//! Headless integration tests verifying `UiGallery` rendering and core interactions.

use std::time::Duration;

use nsh_app_diving::ui::{UiGallery, app_tokens};
use nsh_engine::{EngineConfig, Environment, Idiom, Os, Points, Root, Size};
use nsh_testing::Headless;

fn start_desktop_gallery() -> Headless {
    let config = EngineConfig {
        environment: Environment {
            os: Os::MacOs,
            idiom: Idiom::Desktop,
            ..Default::default()
        },
        viewport: Size::new(Points(1280.0), Points(820.0)),
        ..EngineConfig::new(Root::of::<UiGallery>().tokens(app_tokens()))
    };

    let mut app = Headless::start(config).expect("start headless UiGallery");
    app.set_viewport(Size::new(Points(1280.0), Points(820.0)))
        .expect("set desktop viewport");
    app
}

#[test]
fn gallery_renders_and_switches_tabs() {
    let mut app = start_desktop_gallery();
    assert!(app.shows("Diving"));
    assert!(app.shows("UiGallery"));
    assert!(app.shows("Clicks: 0"));
    assert!(app.shows("Active tab: Layers"));

    app.press("Primary").expect("press Primary button");
    assert!(app.shows("Clicks: 1"));

    app.press("Treemap").expect("press Treemap tab");
    assert!(app.shows("Active tab: Treemap"));
}

#[test]
fn gallery_dialog_and_sheet_open_and_close_on_escape() {
    let mut app = start_desktop_gallery();

    app.press("Open Dialog…").expect("open dialog");
    app.wait_for("Sample inspection modal")
        .expect("wait for dialog body");
    app.press_key("escape").expect("press escape on dialog");
    app.wait_for_gone("Sample inspection modal", Duration::from_secs(2))
        .expect("dialog dismisses on escape");

    app.press("Open Sheet…").expect("open sheet");
    app.wait_for("File preview inspection sheet")
        .expect("wait for sheet body");
    app.press_key("escape").expect("press escape on sheet");
    app.wait_for_gone("File preview inspection sheet", Duration::from_secs(2))
        .expect("sheet dismisses on escape");
}
