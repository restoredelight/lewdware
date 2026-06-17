#[macro_export]
macro_rules! once {
    ($expression:expr) => {{
        static ONCE: std::sync::Once = std::sync::Once::new();

        ONCE.call_once(|| $expression)
    }};
}
