#[macro_export]
macro_rules! ma {
    ($dest:expr, $ma:ident, $params:expr, $candles:expr, $width:expr, $color:expr, $style:expr, $alpha:expr) => {
        $dest.line(
            Line::new(PlotPoints::new(
                $ma::new_over(
                    $params,
                    $candles.iter().map(|c| c.close).collect::<Vec<ValueType>>(),
                )
                .unwrap()
                .into_iter()
                .enumerate()
                .map(|(i, v)| [$candles[i].open_time.timestamp() as ValueType, v])
                .collect(),
            ))
            .stroke(Stroke::new($width, Rgba::from($color).to_opaque().multiply($alpha)))
            .style($style)
            .name(stringify!($ma $params)),
        );
    };
}

#[macro_export]
macro_rules! fib_ma {
    ($dest:expr, $ma:ident, $params:expr, $candles:expr, $width:expr, $color:expr, $alpha:expr) => {
        let values = $candles.iter().map(|c| c.close).collect::<Vec<ValueType>>();

        for len in $params {
            $dest.line(
                Line::new(PlotPoints::new(
                    $ma::new_over(
                        *len,
                        &values,
                    )
                    .unwrap()
                    .into_iter()
                    .enumerate()
                    .map(|(i, v)| [$candles[i].open_time.timestamp() as ValueType, v])
                    .collect(),
                ))
                .stroke(Stroke::new($width, Rgba::from($color).to_opaque().multiply($alpha)))
                .name(stringify!($ma $params)),
            );
        }
    };
}

#[macro_export]
macro_rules! ma_over_values {
    ($dest:expr, $ma:ident, $params:expr, $values:expr, $width:expr, $color:expr, $alpha:expr) => {
        $dest.line(
            Line::new(PlotPoints::new(
                $ma::new_over(
                    $params,
                    $values,
                )
                .unwrap()
                .into_iter()
                .enumerate()
                .map(|(i, v)| [i as f64, v])
                .collect(),
            ))
            .stroke(Stroke::new($width, Rgba::from($color).to_opaque().multiply($alpha)))
            .name(stringify!($ma $params)),
        );
    };
}

#[macro_export]
macro_rules! rsi {
    ($dest:expr, $candles:expr, $width:expr, $color:expr, $alpha:expr) => {
        let mut rsi = RSI::default();
        rsi.ma = MA::RMA(14);
        let mut rsi = RSI::init(rsi, &$candles[0]).unwrap();
        $dest.line(Line::new(PlotPoints::new(
           rsi
            .over(&$candles)
                .into_iter()
                .enumerate()
                .map(|(i, v)| [i as f64, v.value(0)])
                .collect(),
            ))
            .stroke(Stroke::new($width, Rgba::from($color).to_opaque().multiply($alpha)))
            .name(stringify!($ma $params)),
        );
    };
}

#[macro_export]
macro_rules! egui_labeled_value {
    ($ui:expr, $label:expr, $value:expr) => {
        $ui.label($label);
        $ui.label(WidgetText::from(format!("{:?}", $value)).color(Color32::WHITE));
    };
    ($ui:expr, $label:expr, $value:expr, $color:expr) => {
        $ui.label($label);
        $ui.label(WidgetText::from(format!("{:?}", $value)).color($color));
    };
}

#[macro_export]
macro_rules! egui_value {
    ($ui:expr, $value:expr) => {
        $ui.label(WidgetText::from(format!("{:?}", $value)).color(Color32::WHITE));
    };
    ($ui:expr, $value:expr, $color:expr) => {
        $ui.label(WidgetText::from(format!("{:?}", $value)).color($color));
    };
}

#[macro_export]
macro_rules! egui_value2 {
    ($ui:expr, $value:expr) => {
        $ui.label(WidgetText::from(format!("{}", $value)).color(Color32::WHITE));
    };
    ($ui:expr, $value:expr, $color:expr) => {
        $ui.label(WidgetText::from(format!("{}", $value)).color($color));
    };
}

#[macro_export]
macro_rules! egui_formatted_value {
    ($ui:expr, $fmt:expr, $value:expr) => {
        $ui.label(WidgetText::from(format!($fmt, $value)).color(Color32::WHITE));
    };
    ($ui:expr, $fmt:expr, $value:expr, $color:expr) => {
        $ui.label(WidgetText::from(format!($fmt, $value)).color($color));
    };
}

#[macro_export]
macro_rules! egui_formatted_value_with_precision {
    ($ui:expr, $label:expr, $value:expr, $precision:expr) => {
        $ui.label(WidgetText::from(format!("{}: {:.p$}", $label, $value, p = $precision)).color(Color32::WHITE));
    };
    ($ui:expr, $label:expr, $value:expr, $precision:expr, $color:expr) => {
        $ui.label(WidgetText::from(format!("{}: {:.p$}", $label, $value, p = $precision)).color($color));
    };
}

#[macro_export]
macro_rules! egui_string {
    ($ui:expr, $string:expr) => {
        $ui.label(WidgetText::from($string).color(Color32::WHITE));
    };
    ($ui:expr, $string:expr, $color:expr) => {
        $ui.label(WidgetText::from($string).color($color));
    };
}

#[macro_export]
macro_rules! egui_labeled_string {
    ($ui:expr, $label:literal, $string:expr) => {
        $ui.label($label);
        $ui.label(WidgetText::from($string).color(Color32::WHITE));
    };
    ($ui:expr, $label:literal, $string:expr, $color:expr) => {
        $ui.label($label);
        $ui.label(WidgetText::from($string).color($color));
    };
}

#[macro_export]
macro_rules! egui_labeled_widget {
    ($ui:expr, $label:literal, $widget:expr) => {
        $ui.label($label);
        $ui.add($widget);
    };
}

#[macro_export]
macro_rules! egui_label {
    ($ui:expr, $value:expr, $fg_color:expr) => {
        $ui.label(WidgetText::from(format!("{}", $value)).color($fg_color));
    };
    ($ui:expr, $value:expr, $fg_color:expr, $bg_color:expr) => {
        $ui.label(WidgetText::from(format!("{}", $value)).color($fg_color).background_color($bg_color));
    };
}

#[macro_export]
macro_rules! egui_fill_rect {
    ($ui:expr, $border_radius:expr, $color:expr) => {
        $ui.painter().rect_filled($ui.available_rect_before_wrap(), $border_radius, $color)
    };
}
