use external_fonts::{cjk_font, latin_font};

fn main() {
    let use_cjk_profile = std::env::args().any(|argument| argument == "--cjk");
    let selected_font = if use_cjk_profile {
        cjk_font()
    } else {
        latin_font()
    };
    assert!(!selected_font.is_null());
}
