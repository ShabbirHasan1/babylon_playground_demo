use crate::{
    countdown::Countdown,
    f,
    id::{HandleId, OrderId},
    simulator::SimulatorRequestResponse,
    types::{NormalizedOrderType, NormalizedSide, NormalizedTimeInForce, OrderedValueType, Priority, Status},
    util::new_id_u64,
};
use chrono::{DateTime, Utc};
use crossbeam_channel::Sender;
use log::error;
use num_traits::ToPrimitive;
use ordered_float::OrderedFloat;
use std::{
    fmt::{Debug, Display, Formatter},
    time::Duration,
};
use thiserror::Error;
use ustr::Ustr;

#[derive(Error, Clone, Debug, Eq, PartialEq)]
pub enum InstructionError {
    #[error("Instruction state cannot transition from {from:?} to {to:?}")]
    InvalidStateTransition {
        instruction_id: u64,
        from:           InstructionState,
        to:             InstructionState,
    },

    #[error("Instruction cannot be cancelled: {0:#?}")]
    InstructionCannotBeCancelled(ManagedInstruction),
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum ExecutionOutcome {
    Success,
    Failure(String),
}

impl Display for ExecutionOutcome {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecutionOutcome::Success => write!(f, "Success"),
            ExecutionOutcome::Failure(reason) => write!(f, "Failure: {}", reason),
        }
    }
}

impl ExecutionOutcome {
    pub fn is_success(&self) -> bool {
        matches!(self, ExecutionOutcome::Success)
    }

    pub fn is_failure(&self) -> bool {
        matches!(self, ExecutionOutcome::Failure(_))
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Default)]
pub enum InstructionKind {
    #[default]
    Noop,
    CreateOrder,
    CreateOcoOrder,
    CancelReplaceOrder,
    CancelOrder,
    CancelAllOrders,
}

#[derive(Copy, Clone, Eq, PartialEq, Default)]
pub struct InstructionMetadata {
    /// Whether the instruction is a special instruction
    pub is_special: bool,

    /// The kind of instruction
    pub kind: InstructionKind,

    /// The trade ID associated with the instruction
    pub trade_id: Option<HandleId>,

    /// The local order ID associated with the instruction
    pub local_order_id: Option<OrderId>,

    /// The local stop order ID associated with the instruction
    pub local_stop_order_id: Option<OrderId>,

    /// The process ID associated with the instruction
    /// NOTE: This is a reserved field for the process aggregate of multiple instructions
    pub process_id: Option<u64>,
}

impl Debug for InstructionMetadata {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InstructionMetadata")
            .field("is_special", &self.is_special)
            .field("kind", &self.kind)
            .field("trade_id", &self.trade_id)
            .field("local_order_id", &self.local_order_id)
            .field("local_stop_order_id", &self.local_stop_order_id)
            .field("process_id", &self.process_id)
            .finish()
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct InstructionExecutionResult {
    pub instruction_id: u64,
    pub symbol:         Option<Ustr>,
    pub metadata:       InstructionMetadata,
    pub outcome:        Option<ExecutionOutcome>,
    pub executed_at:    Option<DateTime<Utc>>, // When the execution started
    pub execution_time: Option<Duration>,      // How long the execution took
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub enum InstructionState {
    #[default]
    New,
    Running,
    Failed,
    Executed,
}

#[derive(Debug, Copy, Clone)]
pub struct ManagedInstruction {
    pub id:                      u64,
    pub symbol:                  Ustr,
    pub metadata:                InstructionMetadata,
    pub instruction:             Instruction,
    pub contingency_instruction: Option<Instruction>,
    pub priority:                Priority,
    pub retries:                 Countdown<u8>,
    pub retry_cooldown:          Option<Duration>,
    pub state:                   InstructionState,
    pub initialized_at:          DateTime<Utc>,
    pub execute_after:           Option<DateTime<Utc>>, // When the instruction should be executed
    pub executed_at:             Option<DateTime<Utc>>, // When the execution started
    pub execution_time:          Option<Duration>,      // How long the execution took
}

impl PartialEq<Self> for ManagedInstruction {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for ManagedInstruction {}

impl ManagedInstruction {
    pub fn new(
        symbol: Ustr,
        priority: Priority,
        metadata: InstructionMetadata,
        instruction: Instruction,
        contingency_instruction: Option<Instruction>,
        execute_after: Option<DateTime<Utc>>,
    ) -> Self {
        Self {
            id: new_id_u64(),
            symbol,
            metadata,
            instruction,
            contingency_instruction,
            state: InstructionState::New,
            priority,
            retries: Countdown::new(0),
            retry_cooldown: Some(Duration::from_millis(300)),
            initialized_at: Utc::now(),
            execute_after,
            executed_at: None,
            execution_time: None,
        }
    }

    pub fn set_state(&mut self, new_state: InstructionState) -> Result<(), InstructionError> {
        use InstructionState::*;
        let old_state = self.state;

        self.state = match (old_state, new_state) {
            // An instruction can move from Pending to Running or Failed
            (New, Running) => Running,
            (New, Failed) => Failed,

            // Once an instruction is in progress, it might fail or get executed
            (Running, Failed) => Failed,
            (Running, Executed) => Executed,

            // For any other transition, throw an error. If the instruction has Failed or has been Executed, it shouldn't change state again.
            // Also, handle the case where the states are the same.
            (same, state) if same == state => state,

            _ => {
                error!("invalid instruction state transition {:?} -> {:?}", old_state, new_state);
                return Err(InstructionError::InvalidStateTransition {
                    instruction_id: self.id,
                    from:           old_state,
                    to:             new_state,
                });
            }
        };

        if self.state != old_state {
            self.executed_at = match self.state {
                Executed => Some(Utc::now()),
                _ => None,
            };

            self.execution_time = match (self.state, self.executed_at) {
                (Executed, Some(executed_at)) => Some(Duration::from_nanos((executed_at.timestamp_nanos() - self.initialized_at.timestamp_nanos()) as u64)),
                _ => None,
            };

            println!("Instruction state transition {:?} -> {:?}", old_state, new_state);
        }

        Ok(())
    }

    pub fn set_trade_id(&mut self, trade_id: HandleId) {
        self.metadata.trade_id = Some(trade_id);
    }

    pub fn set_local_order_id(&mut self, local_order_id: OrderId) {
        self.metadata.local_order_id = local_order_id.to_u64();
    }

    pub fn attach_to_process(&mut self, process_id: u64) {
        self.metadata.process_id = Some(process_id);
    }

    pub fn kind(&self) -> InstructionKind {
        self.metadata.kind
    }

    pub fn is_cancellation(&self) -> bool {
        self.kind() == InstructionKind::CancelOrder
    }

    pub fn trade_id(&self) -> Option<HandleId> {
        self.metadata.trade_id
    }

    pub fn local_order_id(&self) -> Option<OrderId> {
        self.metadata.local_order_id
    }

    pub fn local_stop_order_id(&self) -> Option<OrderId> {
        self.metadata.local_stop_order_id
    }

    pub fn process_id(&self) -> Option<u64> {
        self.metadata.process_id
    }

    pub fn is_executed(&self) -> bool {
        self.state == InstructionState::Executed
    }

    pub fn is_failed(&self) -> bool {
        self.state == InstructionState::Failed
    }

    pub fn is_new(&self) -> bool {
        self.state == InstructionState::New
    }

    pub fn is_running(&self) -> bool {
        self.state == InstructionState::Running
    }

    pub fn is_pending(&self) -> bool {
        self.state == InstructionState::New || self.state == InstructionState::Running
    }
}

#[derive(Copy, Clone, Default, PartialEq, Eq)]
pub enum Instruction {
    #[default]
    Noop,

    /// Limit buy instruction
    LimitBuy {
        symbol:         Ustr,
        local_order_id: OrderId,
        order_type:     NormalizedOrderType,
        quantity:       OrderedValueType,
        price:          OrderedValueType,
        is_post_only:   bool,
    },

    /// Market buy instruction
    MarketBuy {
        symbol:         Ustr,
        local_order_id: OrderId,
        quantity:       OrderedValueType,
    },

    MarketBuyQuoteQuantity {
        symbol:         Ustr,
        local_order_id: OrderId,
        quote_quantity: OrderedValueType,
    },

    /// Limit sell instruction
    LimitSell {
        symbol:         Ustr,
        local_order_id: OrderId,
        order_type:     NormalizedOrderType,
        quantity:       OrderedValueType,
        price:          OrderedValueType,
        is_post_only:   bool,
    },

    /// Market sell instruction
    MarketSell {
        symbol:         Ustr,
        local_order_id: OrderId,
        quantity:       OrderedValueType,
    },

    /// A generic order creation instruction
    CreateOrder {
        symbol:           Ustr,
        client_order_id:  Option<OrderId>,
        side:             NormalizedSide,
        order_type:       NormalizedOrderType,
        quantity:         OrderedValueType,
        quantity_quote:   Option<OrderedValueType>,      // Optional as it's not required for market orders
        quantity_iceberg: Option<OrderedValueType>,      // Optional as it's not required for market orders
        price:            Option<OrderedValueType>,      // Optional as it's not required for market orders
        time_in_force:    Option<NormalizedTimeInForce>, // Optional, mostly for limit orders
        stop_price:       Option<OrderedValueType>,      // For stop-loss orders
    },

    /// A generic stoploss order creation instruction
    CreateStop {
        symbol:              Ustr,
        order_type:          NormalizedOrderType,
        side:                NormalizedSide,
        local_order_id:      OrderId,
        local_stop_order_id: Option<OrderId>,
        price:               Option<OrderedValueType>,
        stop_price:          OrderedValueType,
        quantity:            OrderedValueType,
    },

    /// A generic OCO order creation instruction
    CreateOcoOrder {
        symbol:                   Ustr,
        list_client_order_id:     Option<OrderId>,
        stop_client_order_id:     OrderId,
        limit_client_order_id:    OrderId,
        side:                     NormalizedSide,
        quantity:                 OrderedValueType,
        price:                    OrderedValueType,
        stop_price:               OrderedValueType,
        stop_limit_price:         Option<OrderedValueType>,
        stop_limit_time_in_force: NormalizedTimeInForce,
    },

    /// A generic order cancellation and replacement instruction
    CancelReplaceOrder {
        symbol:                  Ustr,
        local_original_order_id: OrderId,
        exchange_order_id:       Option<OrderId>,
        cancel_order_id:         Option<OrderId>,
        new_client_order_id:     OrderId,
        new_order_type:          NormalizedOrderType,
        new_time_in_force:       NormalizedTimeInForce,
        new_quantity:            OrderedValueType,
        new_price:               OrderedValueType,
        new_limit_price:         Option<OrderedValueType>,
    },

    /// Cancel an order by local order ID
    CancelOrder {
        symbol:         Ustr,
        local_order_id: OrderId,
    },

    /// Cancel all orders for a symbol
    CancelAll {
        symbol: Ustr,
    },

    /// Finalize an order by local order ID (internal to the system)
    FinalizeOrder {
        symbol:         Ustr,
        local_order_id: OrderId,
        status:         Status,
    },

    CancelStop {
        symbol:              Ustr,
        local_stop_order_id: OrderId,
    },

    CloseTrade {
        symbol:   Ustr,
        trade_id: HandleId,
    },
}

impl Instruction {
    pub fn order_type(&self) -> Option<NormalizedOrderType> {
        match self {
            Instruction::LimitBuy { order_type, .. } => Some(*order_type),
            Instruction::LimitSell { order_type, .. } => Some(*order_type),
            Instruction::MarketBuy { .. } => Some(NormalizedOrderType::Market),
            Instruction::MarketSell { .. } => Some(NormalizedOrderType::Market),
            Instruction::CreateStop { order_type, .. } => Some(*order_type),
            Instruction::CreateOrder { order_type, .. } => Some(*order_type),
            Instruction::CreateOcoOrder { stop_limit_price, .. } if stop_limit_price.is_some() => Some(NormalizedOrderType::Limit),
            Instruction::CreateOcoOrder { .. } => Some(NormalizedOrderType::Market),
            Instruction::CancelReplaceOrder { new_order_type, .. } => Some(*new_order_type),
            _ => None,
        }
    }

    pub fn to_generic(&self) -> Option<Instruction> {
        Some(match self {
            Instruction::Noop => Instruction::Noop,

            Instruction::LimitBuy {
                symbol,
                local_order_id,
                order_type,
                quantity,
                price,
                is_post_only,
            } => Instruction::CreateOrder {
                symbol:           *symbol,
                client_order_id:  Some(*local_order_id),
                side:             NormalizedSide::Buy,
                order_type:       *order_type,
                quantity:         *quantity,
                quantity_quote:   None,
                quantity_iceberg: None,
                price:            Some(*price),
                time_in_force:    None,
                stop_price:       None,
            },

            Instruction::MarketBuy {
                symbol,
                local_order_id,
                quantity,
            } => Instruction::CreateOrder {
                symbol:           *symbol,
                client_order_id:  Some(*local_order_id),
                side:             NormalizedSide::Buy,
                order_type:       NormalizedOrderType::Market,
                quantity:         *quantity,
                quantity_quote:   None,
                quantity_iceberg: None,
                price:            None,
                time_in_force:    None,
                stop_price:       None,
            },

            Instruction::LimitSell {
                symbol,
                local_order_id,
                order_type,
                quantity,
                price,
                is_post_only,
            } => Instruction::CreateOrder {
                symbol:           *symbol,
                client_order_id:  Some(*local_order_id),
                side:             NormalizedSide::Sell,
                order_type:       *order_type,
                quantity:         *quantity,
                quantity_quote:   None,
                quantity_iceberg: None,
                price:            Some(*price),
                time_in_force:    None,
                stop_price:       None,
            },

            Instruction::MarketSell {
                symbol,
                local_order_id,
                quantity,
            } => Instruction::CreateOrder {
                symbol:           *symbol,
                client_order_id:  Some(*local_order_id),
                side:             NormalizedSide::Sell,
                order_type:       NormalizedOrderType::Market,
                quantity:         *quantity,
                quantity_quote:   None,
                quantity_iceberg: None,
                price:            None,
                time_in_force:    None,
                stop_price:       None,
            },

            itself @ Instruction::CreateOrder {
                symbol,
                client_order_id,
                side,
                order_type,
                quantity,
                quantity_quote,
                quantity_iceberg,
                price,
                time_in_force,
                stop_price,
            } => itself.clone(),

            Instruction::CreateOcoOrder {
                symbol,
                list_client_order_id,
                stop_client_order_id,
                limit_client_order_id,
                side,
                quantity,
                price,
                stop_price,
                stop_limit_price,
                stop_limit_time_in_force,
            } => Instruction::CreateOcoOrder {
                symbol:                   *symbol,
                list_client_order_id:     *list_client_order_id,
                stop_client_order_id:     *stop_client_order_id,
                limit_client_order_id:    *limit_client_order_id,
                side:                     *side,
                quantity:                 *quantity,
                price:                    *price,
                stop_price:               *stop_price,
                stop_limit_price:         *stop_limit_price,
                stop_limit_time_in_force: *stop_limit_time_in_force,
            },

            Instruction::CancelReplaceOrder {
                symbol,
                local_original_order_id,
                exchange_order_id,
                cancel_order_id,
                new_client_order_id,
                new_order_type,
                new_time_in_force,
                new_price,
                new_quantity,
                new_limit_price,
            } => Instruction::CancelReplaceOrder {
                symbol:                  *symbol,
                local_original_order_id: *local_original_order_id,
                exchange_order_id:       *exchange_order_id,
                cancel_order_id:         *cancel_order_id,
                new_client_order_id:     *new_client_order_id,
                new_order_type:          *new_order_type,
                new_time_in_force:       *new_time_in_force,
                new_price:               *new_price,
                new_quantity:            *new_quantity,
                new_limit_price:         *new_limit_price,
            },

            _ => return None,
        })
    }
}

impl Debug for Instruction {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Instruction::Noop => {
                write!(f, "Noop")
            }

            Instruction::LimitBuy {
                symbol,
                local_order_id,
                order_type,
                quantity,
                price,
                is_post_only,
            } => f
                .debug_struct("LimitBuy")
                .field("symbol", symbol)
                .field("local_order_id", local_order_id)
                .field("order_type", order_type)
                .field("quantity", &quantity.0)
                .field("price", &price.0)
                .field("is_post_only", is_post_only)
                .finish(),

            Instruction::MarketBuy {
                symbol,
                local_order_id,
                quantity,
            } => f
                .debug_struct("MarketBuy")
                .field("symbol", symbol)
                .field("local_order_id", local_order_id)
                .field("quantity", &quantity.0)
                .finish(),

            Instruction::MarketBuyQuoteQuantity {
                symbol,
                local_order_id,
                quote_quantity,
            } => f
                .debug_struct("MarketBuyQuoteQuantity")
                .field("symbol", symbol)
                .field("local_order_id", local_order_id)
                .field("quote_quantity", &quote_quantity.0)
                .finish(),

            Instruction::LimitSell {
                symbol,
                local_order_id,
                order_type,
                quantity,
                price,
                is_post_only,
            } => f
                .debug_struct("LimitSell")
                .field("symbol", symbol)
                .field("local_order_id", local_order_id)
                .field("order_type", order_type)
                .field("quantity", &quantity.0)
                .field("price", &price.0)
                .field("is_post_only", is_post_only)
                .finish(),

            Instruction::MarketSell {
                symbol,
                local_order_id,
                quantity,
            } => f
                .debug_struct("MarketSell")
                .field("symbol", symbol)
                .field("local_order_id", local_order_id)
                .field("quantity", &quantity.0)
                .finish(),

            Instruction::CreateOrder {
                symbol,
                client_order_id,
                side,
                order_type,
                quantity,
                quantity_quote,
                quantity_iceberg,
                price,
                time_in_force,
                stop_price,
            } => f
                .debug_struct("CreateOrder")
                .field("symbol", symbol)
                .field("client_order_id", client_order_id)
                .field("side", side)
                .field("order_type", order_type)
                .field("quantity", &quantity.0)
                .field("price", &price.unwrap_or(f!(0.0)).0)
                .field("time_in_force", time_in_force)
                .field("stop_price", &stop_price.unwrap_or(f!(0.0)).0)
                .finish(),

            Instruction::CreateOcoOrder {
                symbol,
                list_client_order_id,
                stop_client_order_id,
                limit_client_order_id,
                side,
                quantity,
                price,
                stop_price,
                stop_limit_price,
                stop_limit_time_in_force,
            } => f
                .debug_struct("CreateOcoOrder")
                .field("symbol", symbol)
                .field("list_client_order_id", list_client_order_id)
                .field("stop_client_order_id", stop_client_order_id)
                .field("limit_client_order_id", limit_client_order_id)
                .field("side", side)
                .field("quantity", &quantity.0)
                .field("price", &price.0)
                .field("stop_price", &stop_price.0)
                .field("stop_limit_price", &stop_limit_price.unwrap_or(f!(0.0)).0)
                .field("stop_limit_time_in_force", &stop_limit_time_in_force)
                .finish(),

            Instruction::CancelReplaceOrder {
                symbol,
                local_original_order_id,
                exchange_order_id,
                cancel_order_id,
                new_client_order_id,
                new_order_type,
                new_time_in_force,
                new_price,
                new_quantity,
                new_limit_price,
            } => f
                .debug_struct("CancelReplaceOrder")
                .field("symbol", symbol)
                .field("local_original_order_id", local_original_order_id)
                .field("exchange_order_id", exchange_order_id)
                .field("cancel_order_id", cancel_order_id)
                .field("new_client_order_id", new_client_order_id)
                .field("new_order_type", new_order_type)
                .field("new_time_in_force", new_time_in_force)
                .field("new_price", &new_price.0)
                .field("new_quantity", &new_quantity.0)
                .field("new_limit_price", &new_limit_price.unwrap_or(f!(0.0)).0)
                .finish(),

            Instruction::CancelOrder { symbol, local_order_id } => f
                .debug_struct("CancelOrder")
                .field("symbol", symbol)
                .field("local_order_id", local_order_id)
                .finish(),

            Instruction::CancelAll { symbol } => f.debug_struct("CancelAll").field("symbol", symbol).finish(),

            Instruction::FinalizeOrder {
                symbol,
                local_order_id,
                status,
            } => f
                .debug_struct("FinalizeOrder")
                .field("symbol", symbol)
                .field("local_order_id", local_order_id)
                .field("status", status)
                .finish(),

            Instruction::CreateStop {
                symbol,
                order_type,
                side,
                local_order_id: local_stop_order_id,
                local_stop_order_id: local_stop_limit_order_id,
                stop_price,
                price: limit_price,
                quantity,
            } => f
                .debug_struct("CreateStop")
                .field("symbol", symbol)
                .field("order_type", order_type)
                .field("side", side)
                .field("local_stop_order_id", local_stop_order_id)
                .field("local_stop_limit_order_id", local_stop_limit_order_id)
                .field("stop_price", &stop_price.0)
                .field("limit_price", &limit_price.unwrap_or(f!(0.0)).0)
                .field("quantity", &quantity.0)
                .finish(),

            Instruction::CancelStop { symbol, local_stop_order_id } => f
                .debug_struct("CancelStop")
                .field("symbol", symbol)
                .field("local_stop_order_id", local_stop_order_id)
                .finish(),

            Instruction::CloseTrade { symbol, trade_id } => f.debug_struct("CloseTrade").field("symbol", symbol).field("trade_id", trade_id).finish(),
        }
    }
}
