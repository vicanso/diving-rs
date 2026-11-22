//! Headless integration tests verifying `UiGallery` rendering and core interactions.

use std::time::{Duration, SystemTime};

use nsh_app_diving::ui::{UiGallery, app_tokens};
use nsh_engine::{Clock, EngineConfig, Environment, Idiom, Os, Points, Root, Size};
use nsh_testing::Headless;

fn desktop_gallery_config(clock: Clock) -> EngineConfig {
    EngineConfig {
        environment: Environment {
            os: Os::MacOs,
            idiom: Idiom::Desktop,
            ..Default::default()
        },
        viewport: Size::new(Points(1280.0), Points(820.0)),
        clock,
        ..EngineConfig::new(Root::of::<UiGallery>().tokens(app_tokens()))
    }
}

fn start_desktop_gallery() -> Headless {
    let mut app =
        Headless::start(desktop_gallery_config(Clock::system())).expect("start headless UiGallery");
    app.set_viewport(Size::new(Points(1280.0), Points(820.0)))
        .expect("set desktop viewport");
    app
}

fn start_desktop_gallery_manual() -> Headless {
    let mut app = Headless::start(desktop_gallery_config(Clock::manual(
        SystemTime::UNIX_EPOCH,
    )))
    .expect("start headless UiGallery with manual clock");
    app.first_render().expect("initial render");
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

#[test]
fn gallery_more_actions_menu_opens_on_click_and_triggers_dialog() {
    let mut app = start_desktop_gallery();

    app.press("More actions")
        .expect("press More actions menu trigger");
    app.wait_for("role:menuitem[Open Dialog…]")
        .expect("menu opens showing Open Dialog… item");
    assert_eq!(
        app.screen().matches("role:menuitem[Open Sheet…]"),
        Ok(true),
        "expected Open Sheet… menu item"
    );

    app.press("role:menuitem[Open Dialog…]")
        .expect("choose Open Dialog… from menu");
    app.wait_for("Sample inspection modal")
        .expect("dialog opens from menu item");
    app.press_key("escape").expect("dismiss dialog");
    app.wait_for_gone("Sample inspection modal", Duration::from_secs(2))
        .expect("dialog closes on escape");
}

#[test]
fn gallery_search_input_clears_on_escape() {
    let mut app = start_desktop_gallery();

    app.type_text("Filter files input", "nginx")
        .expect("type filter keyword");
    app.wait_for("label:Clear input")
        .expect("clear button appears when query is non-empty");
    assert_eq!(
        app.screen().input_text("Filter files input"),
        Ok(Some("nginx"))
    );

    app.press_key("escape")
        .expect("press escape in search input");
    app.wait_for_gone("label:Clear input", Duration::from_secs(2))
        .expect("search input clears on escape");
    assert_eq!(app.screen().input_text("Filter files input"), Ok(Some("")));
}

#[test]
fn gallery_menu_trigger_dismisses_and_suppresses_tooltip_while_open() {
    let mut app = start_desktop_gallery_manual();

    app.wait_for("label:More actions")
        .expect("More actions trigger renders");
    assert!(!app.shows("text:More actions"));

    // Hovering the closed trigger shows the tooltip after the hover delay.
    app.hover("label:More actions")
        .expect("hover More actions trigger");
    app.fast_forward(Duration::from_millis(600))
        .expect("advance past tooltip hover delay");
    app.wait_for("text:More actions")
        .expect("tooltip appears on hover when menu is closed");

    // Pressing the trigger opens the menu and hides the tooltip immediately.
    app.press("label:More actions")
        .expect("press More actions trigger while tooltip is visible");
    app.wait_for("role:menuitem[Open Dialog…]")
        .expect("menu opens on trigger click");
    app.wait_for_gone("text:More actions", Duration::from_secs(2))
        .expect("tooltip hides when menu opens");

    // Keeping or re-entering pointer hover while the menu is open must not paint the tooltip.
    app.fast_forward(Duration::from_millis(650))
        .expect("advance clock while pointer stays on trigger");
    assert!(
        !app.shows("text:More actions"),
        "tooltip must stay suppressed while pointer remains on open menu trigger"
    );

    app.hover("Open Dialog…")
        .expect("move pointer off menu trigger while menu is open");
    app.hover("label:More actions")
        .expect("re-hover menu trigger while menu is still open");
    app.fast_forward(Duration::from_millis(650))
        .expect("advance past tooltip delay while menu is open");
    assert!(
        !app.shows("text:More actions"),
        "tooltip must stay suppressed when re-hovering trigger while menu is open"
    );

    // Closing the menu restores normal hover tooltip behavior on a fresh hover.
    app.press_key("escape").expect("dismiss menu with escape");
    app.wait_for_gone("role:menuitem[Open Dialog…]", Duration::from_secs(2))
        .expect("menu closes on escape");
    app.fast_forward(Duration::from_millis(650))
        .expect("settle after menu dismissal");
    assert!(
        !app.shows("text:More actions"),
        "tooltip must not pop back up on menu dismissal"
    );

    app.hover("Open Dialog…")
        .expect("leave trigger after menu closes");
    app.hover("label:More actions")
        .expect("fresh hover on closed trigger");
    app.fast_forward(Duration::from_millis(600))
        .expect("advance past tooltip hover delay after menu closes");
    app.wait_for("text:More actions")
        .expect("tooltip re-enables on fresh hover once menu is closed");
}
