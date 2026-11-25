//! Rado's list on a PocketBook: the InkView and Slint shell around ink-core's model. With the `preview` feature it
//! runs as a desktop window instead (`just preview`).

mod net;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use ink_core::api::Tier as CoreTier;
use ink_core::config;
use ink_core::model::{Heat as CoreHeat, Model, View};
use slint::{ModelRc, SharedString, VecModel};

slint::include_modules!();

/// How often the UI takes the network thread's news.
const TICK: Duration = Duration::from_secs(1);

fn now() -> chrono::DateTime<chrono::FixedOffset> {
    chrono::Local::now().fixed_offset()
}

fn run(config_path: &str) -> Result<(), slint::PlatformError> {
    let config = config::load(config_path, std::fs::read_to_string(config_path));
    let ui = AppWindow::new()?;
    ui.set_config_path(config_path.into());
    let (tx, rx) = mpsc::channel();
    if let Ok(config) = &config {
        net::spawn(config.clone(), tx);
    }
    let model = Rc::new(RefCell::new(Model::new(config)));
    let shown: Rc<RefCell<Option<View>>> = Rc::default();

    let render = Rc::new({
        let (ui, model, shown) = (ui.as_weak(), model.clone(), shown.clone());
        move || {
            let view = model.borrow().view(now());
            if shown.borrow().as_ref() == Some(&view) {
                return;
            }
            if let Some(ui) = ui.upgrade() {
                draw(&ui, &view);
            }
            shown.replace(Some(view));
        }
    });

    ui.on_acknowledge({
        let (model, render) = (model.clone(), render.clone());
        move || {
            model.borrow_mut().acknowledge();
            render();
        }
    });
    ui.on_turn_page({
        let (model, render) = (model.clone(), render.clone());
        move |forward| {
            model.borrow_mut().turn_page(forward);
            render();
        }
    });
    ui.on_exit(|| {
        let _ = slint::quit_event_loop();
    });

    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, TICK, {
        let (model, render) = (model.clone(), render.clone());
        move || {
            for outcome in rx.try_iter() {
                match outcome {
                    Ok(fetched) => model.borrow_mut().fetched(fetched, now()),
                    Err(failure) => model.borrow_mut().failed(failure),
                }
            }
            render();
        }
    });
    render();
    ui.run()
}

fn tier(tier: CoreTier) -> Tier {
    match tier {
        CoreTier::Low => Tier::Low,
        CoreTier::Normal => Tier::Normal,
        CoreTier::Urgent => Tier::Urgent,
    }
}

fn draw(ui: &AppWindow, view: &View) {
    match view {
        View::Setup { problem } => {
            ui.set_mode(Mode::Setup);
            ui.set_message(problem.into());
        }
        View::Waiting { text } => {
            ui.set_mode(Mode::Waiting);
            ui.set_message(text.into());
        }
        View::List(list) => {
            ui.set_mode(Mode::List);
            ui.set_workspace(list.workspace.as_str().into());
            ui.set_reviews(list.reviews as i32);
            ui.set_mine(list.mine as i32);
            ui.set_updated(list.updated.as_str().into());
            let banners: Vec<SharedString> = list.banners.iter().map(|b| b.into()).collect();
            ui.set_banners(ModelRc::new(VecModel::from(banners)));
            ui.set_mood(list.mood.into());
            ui.set_heat(match list.heat {
                CoreHeat::Clear => Heat::Clear,
                CoreHeat::Calm => Heat::Calm,
                CoreHeat::Warm => Heat::Warm,
                CoreHeat::Hot => Heat::Hot,
            });
            ui.set_urgent(list.urgent as i32);
            ui.set_new(list.new as i32);
            ui.set_tally(ModelRc::new(VecModel::from(list.tally.iter().map(|&t| tier(t)).collect::<Vec<_>>())));
            ui.set_tally_more(list.tally_more as i32);
            let items: Vec<Item> = list
                .items
                .iter()
                .map(|i| Item {
                    title: i.title.as_str().into(),
                    age: i.age.as_str().into(),
                    tier: tier(i.tier),
                    new: i.new,
                    re_review: i.re_review,
                    yours: i.yours,
                    draft: i.draft,
                })
                .collect();
            ui.set_items(ModelRc::new(VecModel::from(items)));
            ui.set_page(list.page as i32);
            ui.set_pages(list.pages as i32);
        }
    }
}

/// The config file is the first argument, so the preview can point at its own.
#[cfg(not(feature = "device"))]
fn main() -> Result<(), slint::PlatformError> {
    let path = std::env::args().nth(1).unwrap_or_else(|| config::CONFIG_PATH.into());
    run(&path)
}

#[cfg(feature = "device")]
fn main() {
    use inkview::Event;
    use inkview::bindings::{Inkview, NET_CONNECTED, PANEL_FLAGS_PANEL_NO_FB_OFFSET};
    use inkview::screen::{PixelFormat, Screen, WhichScreen};
    use inkview_slint::{Backend, Event as SlintEvent};

    fn run_on<P: PixelFormat + Copy + Default>(
        screen: Screen<'static, P>,
        events: mpsc::Sender<SlintEvent>,
        ui_events: mpsc::Receiver<SlintEvent>,
    ) {
        slint::platform::set_platform(Box::new(Backend::new(screen, events, ui_events))).expect("the platform is set once");
        if let Err(e) = run(config::CONFIG_PATH) {
            eprintln!("rado-ink: {e}");
        }
    }

    /// Closes the app the InkView way when the UI thread ends, by Exit or by a panic, so the device never keeps a
    /// frozen page.
    struct CloseOnDrop(&'static Inkview);
    impl Drop for CloseOnDrop {
        fn drop(&mut self) {
            // SAFETY: CloseApp takes no arguments; InkView returns to where the app was started from.
            unsafe { self.0.CloseApp() };
        }
    }

    let iv: &'static Inkview = Box::leak(Box::new(inkview::load()));
    let (events, ui_events) = mpsc::channel();
    std::thread::spawn({
        let events = events.clone();
        move || {
            let _close = CloseOnDrop(iv);
            // The framebuffer exists only once InkView has sent Init.
            while !matches!(ui_events.recv(), Ok(SlintEvent::InkviewEvent(Event::Init))) {}
            match WhichScreen::new(iv) {
                WhichScreen::BB8(screen) => run_on(screen, events, ui_events),
                WhichScreen::RGB24(screen) => run_on(screen, events, ui_events),
            }
        }
    });
    inkview::iv_main(iv, move |event| {
        if event == Event::Init {
            // SAFETY: InkView calls on the InkView thread, before the UI thread draws anything.
            unsafe {
                iv.SetPanelType(PANEL_FLAGS_PANEL_NO_FB_OFFSET as i32);
                // NetConnect shows its own dialog and hung once when called off this thread, so it runs here and
                // only while Wi-Fi is down.
                if iv.QueryNetwork() as u32 & NET_CONNECTED == 0 {
                    iv.NetConnect(std::ptr::null());
                }
            }
        }
        events.send(SlintEvent::InkviewEvent(event)).ok()
    });
}
