use crate::{
    cooldown::Cooldown,
    countdown::Countdown,
    f,
    id::{HandleId, HandleOrderKind, SmartId},
    leverage::Leverage,
    order::{Order, OrderMetadata, OrderMode, OrderState, StoplossMetadata},
    plot_action::{
        PlotAction, PlotActionContext, PlotActionError, PlotActionInput, PlotActionInputSequence, PlotActionInputType, PlotActionResult, PlotActionState,
    },
    timeframe::Timeframe,
    trade::{Trade, TradeBuilder, TradeBuilderConfig, TradeError, TradeType},
    trade_closer::{CloserKind, CloserMetadata},
    trader::Trader,
    trigger::{TriggerKind, TriggerMetadata},
    types::{NormalizedOrderType, NormalizedSide, Origin},
    util::{calc_ratio_x_to_y, round_float_to_precision},
};
use chrono::Utc;
use eframe::epaint::Color32;
use egui::{
    plot::{FilledRange, HLine, LineStyle, Orientation, PlotPoint, PlotUi, Text},
    CursorIcon, TextStyle, WidgetText,
};
use ordered_float::OrderedFloat;
use std::{sync::atomic::Ordering, time::Duration};

// TODO: add ability to change by resizing and moving after creation
// TODO: add support for trailing stop but more importantly is to display where it turns on and incremental steps
pub struct TradeBuilderAction {
    /// The the trader context.
    trader: Trader,

    /// The context of this action.
    context: PlotActionContext,

    /// The input sequence for this action. Derived from the trade builder config.
    inputs: PlotActionInputSequence,

    /// The entry order that is being initialized by this action.
    entry: Option<Order>,

    /// The exit order that is being initialized by this action.
    exit: Option<Order>,

    /// The stoploss order that is being initialized by this action.
    stoploss: Option<Order>,

    /// The instance of the trade builder that is being initialized by this action.
    builder: TradeBuilder,

    /// The current state of this action.
    state: PlotActionState,
}

impl TradeBuilderAction {
    pub fn new(context: PlotActionContext, trader: Trader, builder_config: TradeBuilderConfig) -> Result<Self, PlotActionError> {
        if context.primary_keybind.is_none() {
            return Err(PlotActionError::NoPrimaryKeybind);
        }

        // ----------------------------------------------------------------------------
        // Deriving the input sequence from the trade builder config
        // NOTE: The input sequence is starting with the entry point by default
        // ----------------------------------------------------------------------------

        let mut required_inputs = vec![PlotActionInputType::Entry];

        if builder_config.is_closer_required {
            required_inputs.push(PlotActionInputType::Exit);
        }

        // TODO: Add support for stop limit based on the stop order type in the config
        if builder_config.is_stoploss_required {
            required_inputs.push(PlotActionInputType::Stop);
        }

        // ----------------------------------------------------------------------------
        // Initializing the action itself
        // ----------------------------------------------------------------------------

        Ok(Self {
            trader,
            context,
            inputs: PlotActionInputSequence::new(required_inputs),
            entry: None,
            exit: None,
            stoploss: None,
            state: PlotActionState::Active,
            builder: TradeBuilder::new(builder_config),
        })
    }

    fn color_by_input_type(&self, input_type: PlotActionInputType) -> Color32 {
        match input_type {
            PlotActionInputType::Entry => Color32::GOLD,
            PlotActionInputType::Exit => Color32::DARK_GREEN,
            PlotActionInputType::Stop => Color32::LIGHT_RED,
            PlotActionInputType::StopLimit => Color32::RED,
        }
    }

    fn draw_centered_text_in_range(&mut self, plot_ui: &mut PlotUi, point_a: PlotPoint, point_b: PlotPoint, text: String) {
        let bounds = plot_ui.plot_bounds();
        let text_point = PlotPoint::new((bounds.min()[0] + bounds.max()[0]) / 2.0, (point_a.y + point_b.y) / 2.0);
        plot_ui.text(Text::new(text_point, WidgetText::from(text).text_style(TextStyle::Heading).strong()).color(Color32::WHITE));
    }

    fn draw_cursor_line(&mut self, plot_ui: &mut PlotUi) {
        let Some(cursor_pos) = plot_ui.pointer_coordinate() else {
            return;
        };

        let Some(input_type) = self.inputs.get_next_input_type() else {
            return;
        };
        let appraiser = self.context.coin_context.read().appraiser;
        let quote_precision = appraiser.true_quote_precision as usize;
        let base_precision = appraiser.true_base_precision as usize;

        // Drawing the line at the cursor trade
        plot_ui.hline(
            HLine::new(cursor_pos.y, None)
                .color(self.color_by_input_type(input_type))
                .width(1.0)
                .style(LineStyle::dashed_dense())
                .highlight(true),
        );

        // let text_point = PlotPoint::new((self.start_point.x + cursor_pos.x) / 2.0, (net_price + cursor_pos.y) / 2.0);
        // let text = format!("-{:0.2?} -{:0.2?}%", price_delta, price_delta_pc);

        let cursor_plot_point = PlotPoint::new(cursor_pos.x, cursor_pos.y);
        let bounds = plot_ui.plot_bounds();
        let width = bounds.width();
        let height = bounds.height();

        let text_point = PlotPoint::new(cursor_pos.x + width * 0.005, cursor_pos.y + height * 0.005);
        let text = format!("{:0.p$?}", cursor_pos.y, p = quote_precision);

        /*
        // Define the polygon points relative to the bounds
        let points = vec![
            [cursor_pos.x + width * 0.01, cursor_pos.y + height * 0.01],
            [cursor_pos.x + width * 0.01, cursor_pos.y + height * 0.05],
            [cursor_pos.x + width * 0.05, cursor_pos.y + height * 0.05],
            [cursor_pos.x + width * 0.05, cursor_pos.y + height * 0.01],
            [cursor_pos.x + width * 0.01, cursor_pos.y + height * 0.01],
        ];

        // Draw rectangle around the text and above the cursor
        plot_ui.polygon(Polygon::new(PlotPoints::new(points)).color(Color32::WHITE).highlight(true).width(1.0));
         */

        plot_ui.text(Text::new(text_point, WidgetText::from(text).text_style(TextStyle::Heading).strong()).color(Color32::WHITE));
    }

    fn draw_entry(&mut self, plot_ui: &mut PlotUi) {
        match self.inputs.get_point(PlotActionInputType::Entry) {
            Some(PlotActionInput::Entry(Some(entry_point))) => {
                plot_ui.hline(
                    HLine::new(entry_point.y, None)
                        .color(self.color_by_input_type(PlotActionInputType::Entry))
                        .width(1.0)
                        .style(LineStyle::Solid)
                        .highlight(true),
                );
            }

            _ => {
                let Some(cursor_pos) = plot_ui.pointer_coordinate() else {
                    return;
                };

                plot_ui.hline(
                    HLine::new(cursor_pos.y, None)
                        .color(self.color_by_input_type(PlotActionInputType::Entry))
                        .width(1.0)
                        .style(LineStyle::dashed_dense())
                        .highlight(true),
                );
            }
        }
    }

    fn draw_exit(&mut self, plot_ui: &mut PlotUi) {
        let entry_point = match self.inputs.get_point(PlotActionInputType::Entry) {
            Some(PlotActionInput::Entry(Some(entry_point))) => entry_point,
            _ => return,
        };

        let exit_point = match self.inputs.get_point(PlotActionInputType::Exit) {
            Some(PlotActionInput::Exit(Some(exit_point))) => exit_point,
            _ => {
                let Some(cursor_pos) = plot_ui.pointer_coordinate() else {
                    return;
                };

                cursor_pos
            }
        };

        let appraiser = self.context.coin_context.read().appraiser;
        let current_market_price = self.context.coin_context.read().atomic_price.load(Ordering::SeqCst);
        let price_delta = (exit_point.y - entry_point.y).abs();
        let price_delta_ratio = calc_ratio_x_to_y(price_delta, entry_point.y);
        let text = format!("{}%", round_float_to_precision(price_delta_ratio * 100.0, 3));
        self.draw_centered_text_in_range(plot_ui, entry_point, exit_point, text);

        plot_ui.filled_range(
            FilledRange::new(Orientation::Horizontal, true, (false, false), PlotPoint::new(0.0, entry_point.y), PlotPoint::new(0.0, exit_point.y))
                .color(self.color_by_input_type(PlotActionInputType::Exit))
                .fill_alpha(0.1),
        );

        plot_ui.hline(
            HLine::new(exit_point.y, None)
                .color(self.color_by_input_type(PlotActionInputType::Exit))
                .width(1.0)
                .style(LineStyle::Solid)
                .highlight(true),
        );
    }

    fn draw_stop(&mut self, plot_ui: &mut PlotUi) {
        let entry_point = match self.inputs.get_point(PlotActionInputType::Entry) {
            Some(PlotActionInput::Entry(Some(entry_point))) => entry_point,
            _ => return,
        };

        /*
        let (stop_point, stop_limit_price) = match self.inputs.get_point(PlotActionInputType::Stop) {
            Some(PlotActionInput::Stop(Some(stop_point))) => {
                let stop_limit_price = match self.inputs.get_point(PlotActionInputType::Stop) {
                    Some(PlotActionInput::Stop(Some(stop_point))) =>
                        if self.builder.is_shorting() {
                            stop_point.y + (stop_point.y * self.order_config.stoploss.limit_price_offset_ratio.0)
                        } else {
                            stop_point.y - (stop_point.y * self.order_config.stoploss.limit_price_offset_ratio.0)
                        },
                    _ => return,
                };

                (stop_point, stop_limit_price)
            }
            _ => {
                let Some(cursor_pos) = plot_ui.pointer_coordinate() else {
                    return;
                };

                if self.builder.is_shorting() {
                    if cursor_pos.y < entry_point.y {
                        return;
                    }
                } else {
                    if cursor_pos.y > entry_point.y {
                        return;
                    }
                }

                let stop_limit_price = if self.builder.is_shorting() {
                    cursor_pos.y + (cursor_pos.y * self.order_config.stoploss.limit_price_offset_ratio.0)
                } else {
                    cursor_pos.y - (cursor_pos.y * self.order_config.stoploss.limit_price_offset_ratio.0)
                };

                (cursor_pos, stop_limit_price)
            }
        };

        let appraiser = self.context.coin_context.read().appraiser;
        let current_market_price = self.context.coin_context.read().atomic_price.load(Ordering::SeqCst);
        let price_delta = (stop_point.y - entry_point.y).abs();
        let price_delta_ratio = calc_ratio_x_to_y(price_delta, entry_point.y);
        let text = format!("{}%", round_float_to_precision(price_delta_ratio * 100.0, 3));
        self.draw_centered_text_in_range(plot_ui, entry_point, stop_point, text);

        // ----------------------------------------------------------------------------
        // Stop
        // ----------------------------------------------------------------------------

        plot_ui.filled_range(
            FilledRange::new(Orientation::Horizontal, true, (false, false), PlotPoint::new(0.0, entry_point.y), PlotPoint::new(0.0, stop_point.y))
                .color(self.color_by_input_type(PlotActionInputType::Stop))
                .fill_alpha(0.1),
        );

        plot_ui.hline(
            HLine::new(stop_point.y, None)
                .color(self.color_by_input_type(PlotActionInputType::Stop))
                .width(1.0)
                .style(LineStyle::dashed_dense())
                .highlight(true),
        );

        // ----------------------------------------------------------------------------
        // Stop Limit
        // ----------------------------------------------------------------------------

        plot_ui.filled_range(
            FilledRange::new(Orientation::Horizontal, true, (false, false), PlotPoint::new(0.0, stop_limit_price), PlotPoint::new(0.0, stop_point.y))
                .color(self.color_by_input_type(PlotActionInputType::StopLimit))
                .fill_alpha(0.1),
        );

        plot_ui.hline(
            HLine::new(stop_limit_price, None)
                .color(self.color_by_input_type(PlotActionInputType::StopLimit))
                .width(1.0)
                .style(LineStyle::Solid)
                .highlight(true),
        );

         */
    }

    fn build_entry_order(&mut self, trade_id: HandleId) -> Result<Option<Order>, PlotActionError> {
        let appraiser = self.context.coin_context.read().appraiser;
        let current_market_price = f!(self.context.coin_context.read().atomic_price.load(Ordering::SeqCst));
        let max_retry_attempts = self.context.coin_context.read().config.max_retry_attempts.unwrap_or(0);
        let retry_counter = Countdown::new(max_retry_attempts as u8);
        let retry_delay = Duration::from_millis(self.context.coin_context.read().config.retry_delay_ms.unwrap_or(300) as u64);

        let entry_side = match self.builder.config.trade_type {
            TradeType::Long => NormalizedSide::Buy,
            TradeType::Short => NormalizedSide::Sell,
        };

        let entry_price = match self.inputs.get_point(PlotActionInputType::Entry) {
            Some(PlotActionInput::Entry(Some(point))) => {
                f!(appraiser.normalize_quote(point.y))
            }
            _ => return Err(PlotActionError::InvalidEntryPrice),
        };

        // FIXME: This is ugly; look into a better way to do this
        let activator = match self.builder.config.entry_activator.kind {
            TriggerKind::None => TriggerMetadata::none(),
            TriggerKind::Instant => TriggerMetadata::instant(),
            TriggerKind::Delayed => self.builder.config.entry_activator,
            TriggerKind::DelayedZone => self.builder.config.entry_activator,
            TriggerKind::Distance => TriggerMetadata {
                anchor_price: Some(entry_price),
                ..self.builder.config.exit_activator
            },
            TriggerKind::Proximity => {
                // ----------------------------------------------------------------------------
                // NOTE:
                // Using current market price snapshot as an anchor price for proximity activator
                // ----------------------------------------------------------------------------
                TriggerMetadata {
                    anchor_price: Some(current_market_price),
                    target_price: Some(entry_price),
                    ..self.builder.config.exit_activator
                }
            }
        };

        let deactivator = match self.builder.config.entry_deactivator.kind {
            TriggerKind::None => TriggerMetadata::none(),
            TriggerKind::Instant => TriggerMetadata::instant(),
            TriggerKind::Delayed => self.builder.config.entry_deactivator,
            TriggerKind::DelayedZone => self.builder.config.entry_deactivator,
            TriggerKind::Distance => TriggerMetadata {
                anchor_price: Some(entry_price),
                ..self.builder.config.exit_deactivator
            },
            TriggerKind::Proximity => TriggerMetadata {
                anchor_price: Some(current_market_price),
                target_price: Some(entry_price),
                ..self.builder.config.exit_deactivator
            },
        };

        let entry = Order {
            local_order_id: SmartId::new_order_id(trade_id, HandleOrderKind::EntryOrder),
            raw_order_id: None,
            local_stop_order_id: None,
            local_orig_order_id: None,
            local_list_order_id: None,
            exchange_order_id: None,
            trade_id: trade_id,
            mode: OrderMode::Normal,
            pending_mode: None,
            symbol: self.context.symbol,
            atomic_market_price: self.builder.config.atomic_market_price.clone(),
            appraiser,
            timeframe: self.builder.config.timeframe,
            time_in_force: Default::default(),
            side: entry_side,
            order_type: self.builder.config.entry_order_type,
            contingency_order_type: Some(NormalizedOrderType::Market),
            activator: activator.into_trigger()?,
            deactivator: deactivator.into_trigger()?,
            market_price_at_initialization: Some(current_market_price),
            market_price_at_opening: None,
            price: entry_price,
            limit_price: Default::default(),
            initial_quantity: self.builder.config.quantity,
            accumulated_filled_quantity: f!(0.0),
            accumulated_filled_quote: f!(0.0),
            fee_ratio: self.builder.config.fee_ratio,
            retry_counter,
            retry_cooldown: Cooldown::new(retry_delay, 1),
            lifecycle: Default::default(),
            leverage: Leverage::no_leverage(),
            state: OrderState::Idle,
            termination_flag: false,
            cancel_and_idle_flag: false,
            cancel_and_replace_flag: false,
            fills: Default::default(),
            num_updates: 0,
            is_entry_order: true,
            is_created_on_exchange: false,
            is_finished_by_cancel: false,
            pending_instruction: None,
            special_pending_instruction: None,
            metadata: Default::default(),
            initialized_at: Utc::now(),
            opened_at: None,
            updated_at: None,
            trailing_activated_at: None,
            trailing_updated_at: None,
            closed_at: None,
            cancelled_at: None,
            finalized_at: None,
        };

        // NOTE: Ugly but will do for now
        let mut builder = self.builder.clone();
        builder = builder.with_entry(entry.clone());
        self.builder = builder;

        Ok(Some(entry))
    }

    fn build_exit_order(&mut self, trade_id: HandleId) -> Result<Option<Order>, PlotActionError> {
        if !self.builder.config.is_closer_required {
            return Ok(None);
        }

        let appraiser = self.context.coin_context.read().appraiser;
        let current_market_price = f!(self.context.coin_context.read().atomic_price.load(Ordering::SeqCst));
        let max_retry_attempts = self.context.coin_context.read().config.max_retry_attempts.unwrap_or(0);
        let retry_counter = Countdown::new(max_retry_attempts as u8);
        let retry_delay = Duration::from_millis(self.context.coin_context.read().config.retry_delay_ms.unwrap_or(300) as u64);
        let exit_side = match &self.builder.entry {
            None => {
                return Err(PlotActionError::TradeError(TradeError::MissingEntryOrder));
            }
            Some(entry_order) => entry_order.side.opposite(),
        };

        let entry_price = match self.inputs.get_point(PlotActionInputType::Entry) {
            Some(PlotActionInput::Entry(Some(point))) => {
                f!(appraiser.normalize_quote(point.y))
            }
            _ => return Err(PlotActionError::InvalidEntryPrice),
        };

        let exit_price = match self.inputs.get_point(PlotActionInputType::Exit) {
            Some(PlotActionInput::Exit(Some(point))) => {
                f!(appraiser.normalize_quote(point.y))
            }
            _ => return Err(PlotActionError::InvalidExitPrice),
        };

        // FIXME: This is ugly; look into a better way to do this
        let activator = match self.builder.config.exit_activator.kind {
            TriggerKind::None => TriggerMetadata::none(),
            TriggerKind::Instant => TriggerMetadata::instant(),
            TriggerKind::Delayed => self.builder.config.exit_activator,
            TriggerKind::DelayedZone => self.builder.config.exit_activator,
            TriggerKind::Distance => TriggerMetadata {
                anchor_price: Some(entry_price),
                ..self.builder.config.exit_activator
            },
            TriggerKind::Proximity => TriggerMetadata {
                anchor_price: Some(entry_price),
                target_price: Some(exit_price),
                ..self.builder.config.exit_activator
            },
        };

        let deactivator = match self.builder.config.exit_deactivator.kind {
            TriggerKind::None => TriggerMetadata::none(),
            TriggerKind::Instant => TriggerMetadata::instant(),
            TriggerKind::Delayed => self.builder.config.exit_deactivator,
            TriggerKind::DelayedZone => self.builder.config.exit_deactivator,
            TriggerKind::Distance => TriggerMetadata {
                anchor_price: Some(entry_price),
                ..self.builder.config.exit_deactivator
            },
            TriggerKind::Proximity => TriggerMetadata {
                anchor_price: Some(entry_price),
                target_price: Some(exit_price),
                ..self.builder.config.exit_deactivator
            },
        };

        let exit = Order {
            local_order_id: SmartId::new_order_id(trade_id, HandleOrderKind::ExitOrder),
            raw_order_id: None,
            local_stop_order_id: None,
            local_orig_order_id: None,
            local_list_order_id: None,
            exchange_order_id: None,
            trade_id: trade_id,
            mode: OrderMode::Normal,
            pending_mode: None,
            symbol: self.context.symbol,
            atomic_market_price: self.builder.config.atomic_market_price.clone(),
            appraiser,
            timeframe: self.builder.config.timeframe,
            time_in_force: Default::default(),
            side: exit_side,
            order_type: self.builder.config.entry_order_type,
            contingency_order_type: Some(NormalizedOrderType::Market),
            activator: activator.into_trigger()?,
            deactivator: deactivator.into_trigger()?,
            market_price_at_initialization: Some(current_market_price),
            market_price_at_opening: None,
            price: exit_price,
            limit_price: Default::default(),
            initial_quantity: self.builder.config.quantity,
            accumulated_filled_quantity: f!(0.0),
            accumulated_filled_quote: f!(0.0),
            fee_ratio: self.builder.config.fee_ratio,
            retry_counter,
            retry_cooldown: Cooldown::new(retry_delay, 1),
            lifecycle: Default::default(),
            leverage: Leverage::no_leverage(),
            state: OrderState::Idle,
            termination_flag: false,
            cancel_and_idle_flag: false,
            cancel_and_replace_flag: false,
            fills: Default::default(),
            num_updates: 0,
            is_entry_order: false,
            is_created_on_exchange: false,
            is_finished_by_cancel: false,
            pending_instruction: None,
            special_pending_instruction: None,
            initialized_at: Utc::now(),
            opened_at: None,
            updated_at: None,
            trailing_activated_at: None,
            trailing_updated_at: None,
            closed_at: None,
            cancelled_at: None,
            finalized_at: None,
            metadata: Default::default(),
        };

        let mut builder = self.builder.clone();
        builder = builder.with_exit(exit.clone());
        self.builder = builder;

        Ok(Some(exit))
    }

    fn build_stoploss_order(&mut self, trade_id: HandleId) -> Result<Option<Order>, PlotActionError> {
        if !self.builder.config.is_stoploss_required {
            return Ok(None);
        }

        if !matches!(self.builder.config.stoploss_order_type, NormalizedOrderType::StopLoss | NormalizedOrderType::StopLossLimit) {
            return Err(PlotActionError::InvalidStopOrderType(self.builder.config.stoploss_order_type));
        }

        // Need to have an entry order to build a stop order
        let entry = match &self.builder.entry {
            None => {
                return Err(PlotActionError::TradeError(TradeError::MissingEntryOrder));
            }
            Some(entry) => entry,
        };

        let ccx = self.context.coin_context.read();

        let limit_price_offset_ratio = match ccx.config.stop_limit_offset_ratio {
            Some(offset_ratio) => offset_ratio,
            None => return Err(PlotActionError::MissingStopLimitOffsetRatio),
        };

        let appraiser = ccx.appraiser;
        let current_market_price = f!(ccx.atomic_price.load(Ordering::SeqCst));
        let stop_price = match self.inputs.get_point(PlotActionInputType::Stop) {
            Some(PlotActionInput::Stop(Some(point))) => {
                f!(appraiser.normalize_quote(point.y))
            }
            _ => return Err(PlotActionError::InvalidStopPrice),
        };

        let stop_limit_price = if self.builder.config.stoploss_order_type == NormalizedOrderType::StopLossLimit {
            Some(stop_price * (f!(1.0) - limit_price_offset_ratio))
        } else {
            None
        };

        let initial_trailing_step_ratio = ccx.config.initial_trailing_step_ratio.unwrap_or(f!(0.03));
        let trailing_step_ratio = ccx.config.trailing_step_ratio.unwrap_or(f!(0.05));
        let max_retry_attempts = ccx.config.max_retry_attempts.unwrap_or(0);
        let retry_counter = Countdown::new(max_retry_attempts as u8);
        let retry_delay = Duration::from_millis(ccx.config.retry_delay_ms.unwrap_or(300) as u64);
        let side = entry.side.opposite();

        // FIXME: This is ugly; look into a better way to do this
        let activator = match self.builder.config.stoploss_activator.kind {
            TriggerKind::None => TriggerMetadata::none(),
            TriggerKind::Instant => TriggerMetadata::instant(),
            TriggerKind::Delayed => self.builder.config.stoploss_activator,
            TriggerKind::DelayedZone => self.builder.config.stoploss_activator,
            TriggerKind::Distance => TriggerMetadata {
                anchor_price: Some(current_market_price),
                ..self.builder.config.stoploss_activator
            },
            TriggerKind::Proximity => TriggerMetadata {
                anchor_price: Some(current_market_price),
                target_price: Some(stop_price),
                ..self.builder.config.stoploss_activator
            },
        };

        let stoploss = Order {
            local_order_id: SmartId::new_order_id(trade_id, HandleOrderKind::StoplossOrder),
            raw_order_id: None,
            local_stop_order_id: None,
            local_orig_order_id: None,
            local_list_order_id: None,
            exchange_order_id: None,
            trade_id: trade_id,
            mode: OrderMode::Normal,
            pending_mode: None,
            symbol: self.context.symbol,
            atomic_market_price: self.builder.config.atomic_market_price.clone(),
            appraiser,
            timeframe: self.builder.config.timeframe,
            time_in_force: Default::default(),
            side,
            order_type: self.builder.config.stoploss_order_type,
            contingency_order_type: Some(NormalizedOrderType::Market),
            // FIXME: Handle activator and deactivator the same way as entry and exit
            activator: self.builder.config.entry_activator.into_trigger()?,
            deactivator: self.builder.config.entry_deactivator.into_trigger()?,
            market_price_at_initialization: Some(current_market_price),
            market_price_at_opening: None,
            price: stop_price,
            limit_price: stop_limit_price,
            initial_quantity: self.builder.config.quantity,
            accumulated_filled_quantity: f!(0.0),
            accumulated_filled_quote: f!(0.0),
            fee_ratio: self.builder.config.fee_ratio,
            retry_counter,
            retry_cooldown: Cooldown::new(retry_delay, 1),
            lifecycle: Default::default(),
            leverage: Leverage::no_leverage(),
            state: OrderState::Idle,
            termination_flag: false,
            cancel_and_idle_flag: false,
            cancel_and_replace_flag: false,
            fills: Default::default(),
            num_updates: 0,
            is_entry_order: false,
            is_created_on_exchange: false,
            is_finished_by_cancel: false,
            pending_instruction: None,
            special_pending_instruction: None,
            initialized_at: Utc::now(),
            opened_at: None,
            updated_at: None,
            trailing_activated_at: None,
            trailing_updated_at: None,
            closed_at: None,
            cancelled_at: None,
            finalized_at: None,
            metadata: OrderMetadata {
                origin: Origin::Local,
                is_virtual: false,
                limit_price_offset_ratio,
                stoploss: Some(StoplossMetadata {
                    is_trailing_enabled: true,
                    reference_price:     entry.price,
                    initial_stop_price:  stop_price,
                    trailing_step_ratio: trailing_step_ratio,
                }),
            },
        };

        let mut builder = self.builder.clone();
        builder = builder.with_stoploss(stoploss.clone());
        self.builder = builder;

        Ok(Some(stoploss))
    }

    fn build(&mut self) -> Result<Trade, PlotActionError> {
        let trade_id = self.builder.config.specific_trade_id.unwrap_or(SmartId::new_handle_id());

        // IMPORTANT: The trade ID must be set before building the orders
        // FIXME: This is ugly; assigning back to the config is not ideal
        self.builder.config.specific_trade_id = Some(trade_id);

        let entry = self.build_entry_order(trade_id)?;
        let exit = self.build_exit_order(trade_id)?;
        let stoploss = self.build_stoploss_order(trade_id)?;

        let mut builder = self.builder.clone();

        let Some(entry) = entry else {
            return Err(PlotActionError::TradeError(TradeError::MissingEntryOrder));
        };

        let entry_price = entry.price;

        builder = builder.with_entry(entry);

        if self.builder.config.is_closer_required {
            match self.builder.config.closer_kind {
                CloserKind::NoCloser => {
                    // No closer is required, so we just need to add the entry order
                }

                CloserKind::TakeProfit => {
                    let Some(exit) = exit else {
                        return Err(PlotActionError::TradeError(TradeError::MissingExitOrder));
                    };

                    let metadata = CloserMetadata::take_profit(entry_price, false, exit.price);
                    builder = builder.with_closer(metadata.into_closer());
                    builder = builder.with_exit(exit);
                }

                CloserKind::TrendFollowing => {
                    let Some(exit) = exit else {
                        return Err(PlotActionError::TradeError(TradeError::MissingExitOrder));
                    };

                    if self.builder.config.timeframe == Timeframe::NA {
                        return Err(PlotActionError::TradeError(TradeError::InvalidTimeframe(self.builder.config.timeframe)));
                    }

                    let metadata = CloserMetadata::trend_following(entry_price, self.builder.config.timeframe);
                    builder = builder.with_closer(metadata.into_closer());
                    builder = builder.with_exit(exit);
                }
            }
        }

        Ok(builder.build()?)
    }
}

// TODO: Action must require a modifier key to be held down to be active
impl PlotAction for TradeBuilderAction {
    fn compute(&mut self, plot_ui: &mut PlotUi) -> Result<Option<PlotActionResult>, PlotActionError> {
        let Some(primary_keybind) = self.context.primary_keybind else {
            return Ok(None);
        };

        if plot_ui.ctx().input(|i| i.key_down(primary_keybind)) {
            // self.state = PlotActionState::Cancelled;
            return Ok(None);
        }

        // Clicks on the plot will add points to the action
        if let Some(cursor_pos) = plot_ui.pointer_coordinate() {
            // If the cursor is outside the reasonable bounds of the plot, return an error
            if cursor_pos.x < 0.0 || cursor_pos.y < 0.0 {
                return Err(PlotActionError::InvalidInput);
            }

            let is_shorting = self.builder.is_shorting();
            let current_price = self.context.coin_context.read().atomic_price.load(Ordering::SeqCst);

            if plot_ui.ctx().input(|i| i.pointer.primary_clicked()) {
                let Some(input_type) = self.inputs.get_next_input_type() else {
                    return Ok(None);
                };

                // If the value is invalid relative to the current input type, return an error
                match input_type {
                    PlotActionInputType::Entry => {
                        // If the order is shorting, the entry price must be higher than the current price
                        if is_shorting {
                            if cursor_pos.y < current_price {
                                return Err(PlotActionError::InvalidEntryPrice);
                            }
                        } else {
                            if cursor_pos.y > current_price {
                                return Err(PlotActionError::InvalidEntryPrice);
                            }
                        }
                    }

                    PlotActionInputType::Exit => {
                        let entry_price = match self.inputs.get_point(PlotActionInputType::Entry) {
                            Some(PlotActionInput::Entry(Some(entry_point))) => entry_point.y,
                            _ => return Err(PlotActionError::InvalidInput),
                        };

                        // If the order is shorting, the exit price must be lower than the entry price
                        if is_shorting {
                            if cursor_pos.y > entry_price {
                                return Err(PlotActionError::InvalidExitPrice);
                            }
                        } else {
                            if cursor_pos.y < entry_price {
                                return Err(PlotActionError::InvalidExitPrice);
                            }
                        }
                    }

                    PlotActionInputType::Stop => {
                        let entry_price = match self.inputs.get_point(PlotActionInputType::Entry) {
                            Some(PlotActionInput::Entry(Some(entry_point))) => entry_point.y,
                            _ => return Err(PlotActionError::InvalidInput),
                        };

                        // If the order is shorting, the stop price must be higher than the entry price
                        if is_shorting {
                            if cursor_pos.y < entry_price {
                                return Err(PlotActionError::InvalidStopPrice);
                            }
                        } else {
                            if cursor_pos.y > entry_price {
                                return Err(PlotActionError::InvalidStopPrice);
                            }
                        }
                    }

                    PlotActionInputType::StopLimit => {
                        let entry_price = match self.inputs.get_point(PlotActionInputType::Entry) {
                            Some(PlotActionInput::Entry(Some(entry_point))) => entry_point.y,
                            _ => return Err(PlotActionError::InvalidInput),
                        };

                        let stop_price = match self.inputs.get_point(PlotActionInputType::Stop) {
                            Some(PlotActionInput::Stop(Some(stop_point))) => stop_point.y,
                            _ => return Err(PlotActionError::InvalidInput),
                        };

                        // If the order is shorting, the stop price must be higher than the entry price
                        if is_shorting {
                            if cursor_pos.y < stop_price {
                                return Err(PlotActionError::InvalidStopPrice);
                            }
                        } else {
                            if cursor_pos.y > stop_price {
                                return Err(PlotActionError::InvalidStopPrice);
                            }
                        }
                    }
                }

                self.inputs.set_next_input(cursor_pos)?;
            }
        }

        // If the action is complete, return the order metadata as a result
        // IMPORTANT: The order must be validated outside of this action
        if self.inputs.is_complete() && self.state == PlotActionState::Active {
            let trade = self.build()?;
            self.finish();
            return Ok(Some(PlotActionResult::Trade(trade)));
        }

        Ok(None)
    }

    fn draw(&mut self, plot_ui: &mut PlotUi) {
        let Some(cursor_pos) = plot_ui.pointer_coordinate() else {
            return;
        };

        plot_ui.ctx().set_cursor_icon(CursorIcon::None);
        let ccx = self.context.coin_context.read();

        let buy_fee = ccx.config.buy_fee_ratio;
        let sell_fee = ccx.config.sell_fee_ratio;
        let is_zero_maker_fee = ccx.config.is_zero_maker_fee;
        let is_zero_taker_fee = ccx.config.is_zero_taker_fee;
        let is_shorting = self.builder.is_shorting();
        drop(ccx);

        self.draw_cursor_line(plot_ui);
        self.draw_entry(plot_ui);
        self.draw_exit(plot_ui);
        self.draw_stop(plot_ui);
    }

    fn state(&self) -> PlotActionState {
        self.state
    }

    fn context(&self) -> &PlotActionContext {
        &self.context
    }

    fn cancel(&mut self) {
        self.state = PlotActionState::Cancelled;
    }

    fn finish(&mut self) {
        self.state = PlotActionState::Finished;
    }
}
