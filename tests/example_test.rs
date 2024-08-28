#[cfg(test)]
#[embedded_test::tests(executor = esp_hal_embassy::Executor::new())]
mod tests {
    use embassy_sync::blocking_mutex::raw::NoopRawMutex;
    use watchy_rs::sticky_signal::*;

    #[derive(Copy, Clone, PartialEq, Debug)]
    enum TestCommand {
        Start,
        Stop,
    }

    #[test]
    fn test_signal() {
        static SIGNAL: StickySignal<NoopRawMutex, TestCommand> = StickySignal::new();
        SIGNAL.signal(TestCommand::Start);
        assert_eq!(SIGNAL.peek(), Some(TestCommand::Start));
    }

    #[test]
    fn test_reset() {
        static SIGNAL: StickySignal<NoopRawMutex, TestCommand> = StickySignal::new();
        SIGNAL.signal(TestCommand::Start);
        SIGNAL.reset();
        assert_eq!(SIGNAL.peek(), None);
    }

    #[test]
    fn test_try_take() {
        static SIGNAL: StickySignal<NoopRawMutex, TestCommand> = StickySignal::new();
        SIGNAL.signal(TestCommand::Start);
        assert_eq!(SIGNAL.try_take(), Some(TestCommand::Start));
        assert_eq!(SIGNAL.try_take(), None);
    }

    #[test]
    fn test_is_signaled() {
        static SIGNAL: StickySignal<NoopRawMutex, TestCommand> = StickySignal::new();
        assert!(!SIGNAL.is_signaled());
        SIGNAL.signal(TestCommand::Start);
        assert!(SIGNAL.is_signaled());
    }

    #[test]
    fn test_peek() {
        static SIGNAL: StickySignal<NoopRawMutex, TestCommand> = StickySignal::new();
        assert_eq!(SIGNAL.peek(), None);
        SIGNAL.signal(TestCommand::Start);
        assert_eq!(SIGNAL.peek(), Some(TestCommand::Start));
    }
}
