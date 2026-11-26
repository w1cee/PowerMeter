pub type Window = ();

pub fn window_from(_frame: &eframe::Frame) -> Option<Window> {
    Some(())
}

pub fn style_window(_window: Window) {}

pub fn single_instance() -> bool {
    true
}

pub fn info(_window: Option<Window>, text: &str) {
    eprintln!("[info] {text}");
}

pub fn error(_window: Option<Window>, title: &str, text: &str) {
    eprintln!("[error] {title}: {text}");
}

pub fn confirm(_window: Option<Window>, text: &str) -> bool {
    eprintln!("[confirm] {text} -> yes");
    true
}
