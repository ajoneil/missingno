//! Dispatching an automation call on the UI thread: the synchronous tools
//! answer inline; `ui_tree`, `resize_window`, and `screenshot` park their reply
//! and drive a short async pipeline (bounds walk, resize settle, capture) before
//! answering. Per-request state lives in [`App::automation_pending`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use iced::advanced::widget::operate;
use iced::advanced::widget::operation::focusable::focus;
use iced::advanced::widget::operation::scrollable::{AbsoluteOffset, scroll_to as scroll_op};
use iced::widget::Id;
use iced::window::Screenshot;
use iced::{Rectangle, Size, Task, window};
use serde_json::{Value, json};

use missingno_session::tools::{Content, ToolOutcome, base64_encode, outcome_json, text};

use super::bridge::{AutomationBridge, AutomationCall};
use super::capture::{encode_png, physical_crop};
use super::registry::{self, Screen, UiContext};
use super::{CollectBounds, Msg, UiKind, UiNode};
use crate::app::{
    App, DetailSubScreen, FlashState, Game, LoadedGame, Message, PendingAction, Screen as AppScreen,
};

/// The window's minimum content size, matched to `run`'s window settings so a
/// sub-minimum resize can lift it and restore it afterwards.
const MIN_WIDTH: f32 = 1000.0;
const MIN_HEIGHT: f32 = 700.0;
/// How long a resize leg waits for a `Resized` event before settling on the
/// window's actual (possibly WM-clamped) size.
const RESIZE_TIMEOUT: Duration = Duration::from_millis(500);
/// One redraw's grace after a resize settles, so the capture reflects it.
const SETTLE: Duration = Duration::from_millis(100);
/// Compositor grace between unmaximizing and resizing.
const UNMAXIMIZE_GRACE: Duration = Duration::from_millis(100);
/// Client-initiated resizes are dropped on some Wayland compositors (GNOME).
const RESIZE_REFUSED_HINT: &str = "\nnote: the compositor refused the resize; \
    launch the app under XWayland (WAYLAND_DISPLAY= missingno) for exact-size captures";

/// A parked automation reply and the state its async pipeline carries.
pub enum PendingReply {
    /// A `ui_tree` reply whose described nodes await the bounds walk that says
    /// which are on screen.
    Tree {
        reply: Sender,
        nodes: Vec<(String, UiKind, String, bool)>,
        screen: &'static str,
        window: (Option<f32>, Option<f32>),
        scale: Option<f32>,
    },
    /// A `resize_window` reply awaiting the window settling to a new size.
    Resize {
        reply: Sender,
        target: Size,
        restore_min: bool,
    },
    /// A `screenshot` reply moving through resize → settle → capture.
    Shot {
        reply: Sender,
        crop: CropSpec,
        save_path: Option<PathBuf>,
        restore_min: bool,
        resize_target: Option<Size>,
        stage: ShotStage,
    },
}

type Sender = std::sync::mpsc::Sender<Value>;

/// What a screenshot crops to. An `element_id` resolves to a `Region` once its
/// bounds are walked at capture time.
pub enum CropSpec {
    Full,
    Region(Rectangle),
    Element(String),
}

/// Where a parked screenshot is in its pipeline.
pub enum ShotStage {
    AwaitingResize,
    Settling,
    Capturing,
}

pub(in crate::app) fn handle(app: &mut App, msg: Msg) -> Task<Message> {
    match msg {
        Msg::Bridge(AutomationBridge::Ready(sink)) => {
            app.automation_sink.set(sink);
            Task::none()
        }
        Msg::Bridge(AutomationBridge::Call(call)) => dispatch(app, call),
        Msg::TreeCollected { request, pairs } => finish_tree(app, request, pairs),
        Msg::ResizeTimeout { request } => resize_timeout(app, request),
        Msg::ShotSettled { request } => shot_capture(app, request),
        Msg::ShotBounds { request, pairs } => finish_bounds(app, request, pairs),
        Msg::ShotCaptured { request, shot } => finish_shot(app, request, shot),
        Msg::ScaleCached(scale) => {
            app.window_scale = Some(scale);
            Task::none()
        }
    }
}

/// A one-shot task to re-read the window's scale factor into the app cache.
pub(in crate::app) fn query_scale() -> Task<Message> {
    window::latest().and_then(|id| {
        window::scale_factor(id).map(|scale| Message::Automation(Msg::ScaleCached(scale)))
    })
}

fn dispatch(app: &mut App, call: AutomationCall) -> Task<Message> {
    match call.tool.as_str() {
        "status" => {
            reply(&call, text(status_text(app)));
            Task::none()
        }
        "ui_tree" => ui_tree(app, call),
        "activate" => activate(app, call),
        "set_text" => set_text(app, call),
        "scroll_to" => scroll_to(app, call),
        "resize_window" => resize_window(app, call),
        "screenshot" => screenshot(app, call),
        other => {
            reply(&call, Err(format!("unknown tool: {other}")));
            Task::none()
        }
    }
}

fn ui_tree(app: &mut App, call: AutomationCall) -> Task<Message> {
    let ctx = app.ui_context();
    let ids = registry::enumerate(&ctx);
    let nodes: Vec<(String, UiKind, String, bool)> = ids
        .iter()
        .filter_map(|id| {
            registry::describe(&ctx, id)
                .map(|(kind, label)| (id.clone(), kind, label, registry::enabled(&ctx, id)))
        })
        .collect();

    let request = next_request(app);
    app.automation_pending.insert(
        request,
        PendingReply::Tree {
            reply: call.reply,
            nodes,
            screen: screen_name(ctx.screen),
            window: (app.settings.window_width, app.settings.window_height),
            scale: app.window_scale,
        },
    );

    operate(CollectBounds::new(&ids))
        .map(move |pairs| Message::Automation(Msg::TreeCollected { request, pairs }))
}

fn finish_tree(app: &mut App, request: u64, pairs: Vec<(String, Rectangle)>) -> Task<Message> {
    let Some(PendingReply::Tree {
        reply,
        nodes,
        screen,
        window,
        scale,
    }) = app.automation_pending.remove(&request)
    else {
        return Task::none();
    };
    let bounds: HashMap<String, Rectangle> = pairs.into_iter().collect();
    let nodes: Vec<Value> = nodes
        .iter()
        .filter_map(|(id, kind, label, enabled)| {
            bounds.get(id).map(|rect| {
                UiNode {
                    id: id.clone(),
                    kind: *kind,
                    label: label.clone(),
                    bounds: *rect,
                    enabled: *enabled,
                }
                .to_json()
            })
        })
        .collect();
    let body = json!({
        "screen": screen,
        "window": window_json(window, scale),
        "nodes": nodes,
    });
    let _ = reply.send(outcome_json(text(body.to_string())));
    Task::none()
}

// --- resize_window -----------------------------------------------------------

fn resize_window(app: &mut App, call: AutomationCall) -> Task<Message> {
    let (Some(width), Some(height)) = (f32_arg(&call.args, "width"), f32_arg(&call.args, "height"))
    else {
        reply(
            &call,
            Err("'width' and 'height' (numbers) are required".into()),
        );
        return Task::none();
    };
    if width <= 0.0 || height <= 0.0 {
        reply(&call, Err("width and height must be positive".into()));
        return Task::none();
    }
    let target = Size::new(width, height);
    let restore_min = below_min(target);
    let request = next_request(app);
    app.automation_pending.insert(
        request,
        PendingReply::Resize {
            reply: call.reply,
            target,
            restore_min,
        },
    );
    Task::batch([
        apply_resize(target, restore_min),
        sleep_then(RESIZE_TIMEOUT, Msg::ResizeTimeout { request }),
    ])
}

/// A resize leg's fallback fired: if the request is still waiting, settle it on
/// the window's actual size (`Resized` may never come if the WM clamped).
fn resize_timeout(app: &mut App, request: u64) -> Task<Message> {
    match app.automation_pending.get(&request) {
        Some(PendingReply::Resize { .. }) => {
            let actual = current_window_size(app);
            complete_resize(app, request, actual)
        }
        Some(PendingReply::Shot {
            stage: ShotStage::AwaitingResize,
            ..
        }) => begin_settle(app, request),
        _ => Task::none(),
    }
}

/// A `Resized` event: settle any parked resize/screenshot whose target it meets.
pub(in crate::app) fn on_window_resized(app: &mut App, size: Size) -> Task<Message> {
    let ready: Vec<u64> = app
        .automation_pending
        .iter()
        .filter_map(|(request, pending)| match pending {
            PendingReply::Resize { target, .. } if size_matches(*target, size) => Some(*request),
            PendingReply::Shot {
                stage: ShotStage::AwaitingResize,
                resize_target: Some(target),
                ..
            } if size_matches(*target, size) => Some(*request),
            _ => None,
        })
        .collect();

    let tasks: Vec<Task<Message>> = ready
        .into_iter()
        .map(|request| match app.automation_pending.get(&request) {
            Some(PendingReply::Resize { .. }) => complete_resize(app, request, size),
            _ => begin_settle(app, request),
        })
        .collect();
    Task::batch(tasks)
}

fn complete_resize(app: &mut App, request: u64, actual: Size) -> Task<Message> {
    let Some(PendingReply::Resize {
        reply,
        restore_min,
        target,
    }) = app.automation_pending.remove(&request)
    else {
        return Task::none();
    };
    let mut summary = format!(
        "window is now {}x{} logical",
        actual.width as u32, actual.height as u32
    );
    if !size_matches(target, actual) {
        summary.push_str(RESIZE_REFUSED_HINT);
    }
    let _ = reply.send(outcome_json(text(summary)));
    if restore_min {
        restore_min_task()
    } else {
        Task::none()
    }
}

// --- screenshot --------------------------------------------------------------

fn screenshot(app: &mut App, call: AutomationCall) -> Task<Message> {
    let args = &call.args;
    if args.get("region").is_some() && args.get("element_id").is_some() {
        reply(
            &call,
            Err("'region' and 'element_id' are mutually exclusive".into()),
        );
        return Task::none();
    }
    let crop = if args.get("region").is_some() {
        match parse_region(args) {
            Ok(rect) => CropSpec::Region(rect),
            Err(error) => {
                reply(&call, Err(error));
                return Task::none();
            }
        }
    } else if let Some(id) = str_arg(args, "element_id") {
        CropSpec::Element(id)
    } else {
        CropSpec::Full
    };

    let resize_target = match resize_target(app, args) {
        Ok(target) => target,
        Err(error) => {
            reply(&call, Err(error));
            return Task::none();
        }
    };
    let save_path = str_arg(args, "path").map(PathBuf::from);
    let restore_min = resize_target.map(below_min).unwrap_or(false);
    let request = next_request(app);

    let (stage, resize_leg) = match resize_target {
        Some(target) => (
            ShotStage::AwaitingResize,
            Some(Task::batch([
                apply_resize(target, restore_min),
                sleep_then(RESIZE_TIMEOUT, Msg::ResizeTimeout { request }),
            ])),
        ),
        None => (ShotStage::Capturing, None),
    };
    app.automation_pending.insert(
        request,
        PendingReply::Shot {
            reply: call.reply,
            crop,
            save_path,
            restore_min,
            resize_target,
            stage,
        },
    );
    // No resize leg: skip straight to capture.
    resize_leg.unwrap_or_else(|| shot_capture(app, request))
}

/// Start the post-resize redraw settle for a parked screenshot.
fn begin_settle(app: &mut App, request: u64) -> Task<Message> {
    if let Some(PendingReply::Shot { stage, .. }) = app.automation_pending.get_mut(&request) {
        *stage = ShotStage::Settling;
        sleep_then(SETTLE, Msg::ShotSettled { request })
    } else {
        Task::none()
    }
}

/// The settle elapsed (or there was no resize): capture now, walking element
/// bounds first when cropping to one.
fn shot_capture(app: &mut App, request: u64) -> Task<Message> {
    let Some(PendingReply::Shot { crop, stage, .. }) = app.automation_pending.get_mut(&request)
    else {
        return Task::none();
    };
    *stage = ShotStage::Capturing;
    match crop {
        CropSpec::Element(id) => {
            let ids = [id.clone()];
            operate(CollectBounds::new(&ids))
                .map(move |pairs| Message::Automation(Msg::ShotBounds { request, pairs }))
        }
        _ => capture_task(request),
    }
}

fn finish_bounds(app: &mut App, request: u64, pairs: Vec<(String, Rectangle)>) -> Task<Message> {
    match pairs.into_iter().next() {
        Some((_, rect)) => {
            if let Some(PendingReply::Shot { crop, .. }) = app.automation_pending.get_mut(&request)
            {
                *crop = CropSpec::Region(rect);
                capture_task(request)
            } else {
                Task::none()
            }
        }
        None => fail_shot(app, request, "element is not on screen".into()),
    }
}

fn finish_shot(app: &mut App, request: u64, shot: Screenshot) -> Task<Message> {
    let Some(PendingReply::Shot {
        reply,
        crop,
        save_path,
        restore_min,
        resize_target,
        ..
    }) = app.automation_pending.remove(&request)
    else {
        return Task::none();
    };
    let _ = reply.send(outcome_json(render_shot(
        &shot,
        &crop,
        save_path.as_deref(),
        resize_target,
    )));
    if restore_min {
        restore_min_task()
    } else {
        Task::none()
    }
}

fn fail_shot(app: &mut App, request: u64, message: String) -> Task<Message> {
    if let Some(PendingReply::Shot {
        reply, restore_min, ..
    }) = app.automation_pending.remove(&request)
    {
        let _ = reply.send(outcome_json(Err(message)));
        if restore_min {
            return restore_min_task();
        }
    }
    Task::none()
}

/// Crop the capture, encode a PNG, optionally write it, and build the result.
fn render_shot(
    shot: &Screenshot,
    crop: &CropSpec,
    save_path: Option<&Path>,
    resize_target: Option<Size>,
) -> ToolOutcome {
    let capture = (shot.size.width, shot.size.height);
    let cropped = match crop {
        CropSpec::Full => shot.clone(),
        CropSpec::Region(rect) => {
            let (x, y, width, height) = physical_crop(
                (rect.x, rect.y, rect.width, rect.height),
                shot.scale_factor,
                capture,
            )
            .ok_or("the crop rect has no overlap with the window")?;
            shot.crop(Rectangle {
                x,
                y,
                width,
                height,
            })
            .map_err(|error| format!("crop failed: {error}"))?
        }
        CropSpec::Element(_) => return Err("element bounds were not resolved".into()),
    };
    let png = encode_png(cropped.size.width, cropped.size.height, &cropped.rgba)?;
    if let Some(path) = save_path {
        std::fs::write(path, &png)
            .map_err(|error| format!("writing {}: {error}", path.display()))?;
    }
    let mut summary = format!(
        "captured {}x{} physical px at scale {}",
        cropped.size.width, cropped.size.height, shot.scale_factor
    );
    let window_logical = Size::new(
        shot.size.width as f32 / shot.scale_factor,
        shot.size.height as f32 / shot.scale_factor,
    );
    if let Some(target) = resize_target
        && !size_matches(target, window_logical)
    {
        summary.push_str(RESIZE_REFUSED_HINT);
    }
    if let Some(path) = save_path {
        let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        summary.push_str(&format!("\nwritten to {}", absolute.display()));
    }
    Ok(vec![
        Content::Image {
            data: base64_encode(&png),
            mime_type: "image/png".into(),
        },
        Content::Text(summary),
    ])
}

fn activate(app: &mut App, call: AutomationCall) -> Task<Message> {
    let Some(id) = str_arg(&call.args, "id") else {
        reply(&call, Err("'id' (string) is required".into()));
        return Task::none();
    };
    let ctx = app.ui_context();
    match registry::activation(&ctx, &id) {
        Some(message) => {
            reply(&call, text(format!("activated {id}")));
            app.update(message)
        }
        None => {
            reply(&call, Err(format!("{id} has no activation here")));
            Task::none()
        }
    }
}

fn set_text(app: &mut App, call: AutomationCall) -> Task<Message> {
    let (Some(id), Some(new_text)) = (str_arg(&call.args, "id"), str_arg(&call.args, "text"))
    else {
        reply(&call, Err("'id' and 'text' (strings) are required".into()));
        return Task::none();
    };
    let ctx = app.ui_context();
    match registry::text_change(&ctx, &id, new_text) {
        Some(message) => {
            reply(&call, text(format!("set text on {id}")));
            let focus_input = operate(focus::<Message>(Id::from(id)));
            Task::batch([focus_input, app.update(message)])
        }
        None => {
            reply(&call, Err(format!("{id} takes no text")));
            Task::none()
        }
    }
}

fn scroll_to(app: &mut App, call: AutomationCall) -> Task<Message> {
    let Some(id) = str_arg(&call.args, "id") else {
        reply(&call, Err("'id' (string) is required".into()));
        return Task::none();
    };
    // Only offer scroll on a scrollable this context knows.
    let ctx = app.ui_context();
    if registry::describe(&ctx, &id).is_none() {
        reply(&call, Err(format!("no node {id} here")));
        return Task::none();
    }
    let x = f32_arg(&call.args, "x").unwrap_or(0.0);
    let y = f32_arg(&call.args, "y").unwrap_or(0.0);
    reply(&call, text(format!("scrolled {id}")));
    let offset = AbsoluteOffset {
        x: Some(x),
        y: Some(y),
    };
    operate(scroll_op::<Message>(Id::from(id), offset))
}

// --- helpers -----------------------------------------------------------------

/// Post a tool outcome back to the waiting socket client.
fn reply(call: &AutomationCall, outcome: ToolOutcome) {
    let _ = call.reply.send(outcome_json(outcome));
}

fn str_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(str::to_string)
}

fn f32_arg(args: &Value, key: &str) -> Option<f32> {
    args.get(key)
        .and_then(Value::as_f64)
        .map(|value| value as f32)
}

fn next_request(app: &mut App) -> u64 {
    let request = app.automation_next_request;
    app.automation_next_request += 1;
    request
}

fn below_min(size: Size) -> bool {
    size.width < MIN_WIDTH || size.height < MIN_HEIGHT
}

/// Whether an actual window size is within a pixel or two of a requested one.
fn size_matches(target: Size, actual: Size) -> bool {
    (target.width - actual.width).abs() <= 2.0 && (target.height - actual.height).abs() <= 2.0
}

fn current_window_size(app: &App) -> Size {
    Size::new(
        app.settings.window_width.unwrap_or(MIN_WIDTH),
        app.settings.window_height.unwrap_or(MIN_HEIGHT),
    )
}

/// The requested target size for a screenshot's optional resize leg: `None`
/// unless a width or height is given, filling the missing axis from the current
/// window size.
fn resize_target(app: &App, args: &Value) -> Result<Option<Size>, String> {
    let (width, height) = (f32_arg(args, "width"), f32_arg(args, "height"));
    if width.is_none() && height.is_none() {
        return Ok(None);
    }
    if width.is_some_and(|w| w <= 0.0) || height.is_some_and(|h| h <= 0.0) {
        return Err("width and height must be positive".into());
    }
    let current = current_window_size(app);
    Ok(Some(Size::new(
        width.unwrap_or(current.width),
        height.unwrap_or(current.height),
    )))
}

fn parse_region(args: &Value) -> Result<Rectangle, String> {
    let region = args.get("region").ok_or("'region' object is required")?;
    let width = f32_arg(region, "width").ok_or("region needs 'width'")?;
    let height = f32_arg(region, "height").ok_or("region needs 'height'")?;
    if width <= 0.0 || height <= 0.0 {
        return Err("region width and height must be positive".into());
    }
    Ok(Rectangle {
        x: f32_arg(region, "x").unwrap_or(0.0),
        y: f32_arg(region, "y").unwrap_or(0.0),
        width,
        height,
    })
}

/// Resize the window to `target`, lifting the minimum size first when the target
/// is below it.
fn apply_resize(target: Size, drop_min: bool) -> Task<Message> {
    window::latest().and_then(move |id| {
        let mut tasks = Vec::new();
        // A maximized window ignores resize requests, and the compositor applies
        // the unmaximize asynchronously — the resize must arrive after it.
        tasks.push(window::maximize::<Message>(id, false));
        if drop_min {
            tasks.push(window::set_min_size::<Message>(id, None));
        }
        Task::batch(tasks).chain(
            Task::future(async move {
                smol::Timer::after(UNMAXIMIZE_GRACE).await;
                id
            })
            .then(move |id| window::resize::<Message>(id, target)),
        )
    })
}

fn restore_min_task() -> Task<Message> {
    window::latest()
        .and_then(|id| window::set_min_size::<Message>(id, Some(Size::new(MIN_WIDTH, MIN_HEIGHT))))
}

fn capture_task(request: u64) -> Task<Message> {
    window::latest().and_then(move |id| {
        window::screenshot(id)
            .map(move |shot| Message::Automation(Msg::ShotCaptured { request, shot }))
    })
}

/// A task that sleeps, then raises `message` — the app's async pause facility
/// (iced's smol executor drives the timer), used for resize and settle waits.
fn sleep_then(delay: Duration, message: Msg) -> Task<Message> {
    Task::future(async move {
        smol::Timer::after(delay).await;
        Message::Automation(message)
    })
}

fn window_json(window: (Option<f32>, Option<f32>), scale: Option<f32>) -> Value {
    json!({ "width": window.0, "height": window.1, "scale": scale })
}

fn screen_name(screen: Screen) -> &'static str {
    match screen {
        Screen::Library => "library",
        Screen::GameDetail => "game_detail",
        Screen::CartridgeActions => "cartridge_actions",
        Screen::FlashCartridge => "flash_cartridge",
        Screen::HomebrewBrowser => "homebrew_browser",
        Screen::ScreenshotGallery => "screenshot_gallery",
        Screen::Settings => "settings",
        Screen::Emulator => "emulator",
    }
}

fn status_text(app: &App) -> String {
    let ctx = app.ui_context();
    let loaded = matches!(app.game, Game::Loaded(_));
    let (width, height) = (app.settings.window_width, app.settings.window_height);
    let size = match (width, height) {
        (Some(w), Some(h)) => format!("{w}x{h} logical"),
        _ => "unknown".to_string(),
    };
    let scale = match app.window_scale {
        Some(scale) => scale.to_string(),
        None => "unknown".to_string(),
    };
    format!(
        "screen: {}\nwindow: {size}\nscale: {scale}\ngame loaded: {loaded}\nrunning: {}\n\
         debugger: {}\n(Every screen exposes its navigation and primary actions; \
         per-item lists — activity log entries, homebrew and gallery thumbnails — \
         are not individually registered.)",
        screen_name(ctx.screen),
        ctx.running,
        ctx.is_debugger,
    )
}

impl App {
    /// The narrow view of app state the registry reads.
    fn ui_context(&self) -> UiContext {
        let screen = match &self.screen {
            AppScreen::Library { .. } => Screen::Library,
            AppScreen::ViewingGame {
                sub_screen: DetailSubScreen::Detail { .. },
                ..
            } => Screen::GameDetail,
            AppScreen::ViewingGame {
                sub_screen: DetailSubScreen::CartridgeActions { .. },
                ..
            } => Screen::CartridgeActions,
            AppScreen::ViewingGame {
                sub_screen: DetailSubScreen::FlashCartridge { .. },
                ..
            } => Screen::FlashCartridge,
            AppScreen::ViewingGame {
                sub_screen: DetailSubScreen::ScreenshotGallery { .. },
                ..
            } => Screen::ScreenshotGallery,
            AppScreen::HomebrewBrowser { .. } => Screen::HomebrewBrowser,
            AppScreen::Settings { .. } => Screen::Settings,
            AppScreen::Emulator => Screen::Emulator,
        };
        let games = if matches!(screen, Screen::Library) {
            self.store
                .summaries_sorted(
                    self.settings.library_sort,
                    &self.library_search,
                    self.library_filter,
                )
                .iter()
                .map(|summary| {
                    (
                        summary.entry.sha1.clone(),
                        summary.entry.display_title().to_string(),
                    )
                })
                .collect()
        } else {
            Vec::new()
        };
        let settings_section = match &self.screen {
            AppScreen::Settings { section, .. } => *section,
            _ => crate::app::settings::view::Section::default(),
        };
        let settings_controls = match &self.screen {
            AppScreen::Settings { controls, .. } => controls.clone(),
            _ => Default::default(),
        };
        let settings_pointer_knob =
            crate::app::settings::view::page_pointer_knob(settings_controls.page, &self.settings);
        let viewing_sha1 = self.viewing_sha1().map(str::to_string);
        let (detail_has_rom, detail_game_loaded, detail_cartridge_actions) =
            match viewing_sha1.as_deref() {
                Some(sha1) if matches!(screen, Screen::GameDetail) => self.detail_affordances(sha1),
                _ => (false, false, false),
            };
        let flash_in_progress = matches!(
            &self.screen,
            AppScreen::ViewingGame {
                sub_screen: DetailSubScreen::FlashCartridge {
                    flash_state: FlashState::InProgress(_),
                },
                ..
            }
        );
        let homebrew_entry_selected = matches!(
            &self.screen,
            AppScreen::HomebrewBrowser { state } if state.selected_slug.is_some()
        );
        // The Controllers pick lists exist only while that panel is open on the
        // play screen.
        let controllers = match &self.game {
            Game::Loaded(LoadedGame::Emulator(emulator))
                if matches!(screen, Screen::Emulator)
                    && emulator.shows_panel(crate::app::emulator::PlayPanel::Controllers) =>
            {
                let seating = self.controller_seating();
                (!seating.ports.is_empty()).then_some(seating)
            }
            _ => None,
        };
        // The Display panel's rows likewise exist only while it is open.
        let display = match &self.game {
            Game::Loaded(LoadedGame::Emulator(emulator))
                if matches!(screen, Screen::Emulator)
                    && emulator.shows_panel(crate::app::emulator::PlayPanel::Display) =>
            {
                Some(emulator.display_options())
            }
            _ => None,
        };
        UiContext {
            screen,
            running: self.running(),
            is_debugger: matches!(self.game, Game::Loaded(LoadedGame::Debugger(_))),
            debugger_enabled: self.debugger_enabled,
            menu_open: self.menu_open,
            confirm_accept_label: self.pending_action.as_ref().map(confirm_accept_label),
            games,
            settings_section,
            settings_controls,
            settings_pointer_knob,
            settings_display: self.settings.display_options(),
            allow_external_clients: self.settings.allow_external_clients,
            allow_ui_automation: self.settings.allow_ui_automation,
            library_layout: self.settings.library_layout,
            homebrew_available: self.settings.internet_enabled
                && self.settings.homebrew_hub_enabled,
            homebrew_entry_selected,
            viewing_sha1,
            detail_has_rom,
            detail_game_loaded,
            detail_cartridge_actions,
            flash_in_progress,
            controllers,
            display,
        }
    }

    /// The detail screen's conditional affordances for a viewed game: whether it
    /// has a ROM on disk, is currently loaded, and offers cartridge actions.
    fn detail_affordances(&self, sha1: &str) -> (bool, bool, bool) {
        let Some(entry) = self.store.entry(sha1) else {
            return (false, false, false);
        };
        let has_rom = entry.rom_paths.iter().any(|path| path.exists());
        let game_loaded = self
            .current_game
            .as_ref()
            .map(|current| current.entry.sha1 == sha1 && matches!(self.game, Game::Loaded(_)))
            .unwrap_or(false);
        let cartridge_actions = self.inserted_cartridge().is_some_and(|cart| {
            let matches = entry
                .header_title
                .as_ref()
                .is_some_and(|title| title == &cart.title);
            matches || cart.flashable()
        });
        (has_rom, game_loaded, cartridge_actions)
    }
}

/// The confirm button's label for a pending action, matching the shell's dialog.
fn confirm_accept_label(action: &PendingAction) -> String {
    match action {
        PendingAction::SwitchGame(_) => "Close Game",
        PendingAction::CloseApp => "Quit",
        PendingAction::ResetEmulator => "Reset",
        PendingAction::StopGame => "Stop",
        PendingAction::RemoveGameFromLibrary => "Remove",
    }
    .to_string()
}
