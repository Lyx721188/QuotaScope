use std::time::Duration;
use windows_reactor::SlotsControl;
use windows_reactor::*;
#[path = "../../src/fonts.rs"]
mod fonts;

struct Probe {
    _task: ComponentTask,
    keeper: bool,
    rounds: u8,
    title: &'static str,
}
impl Component for Probe {
    type Input = (u64, &'static str);
    type Message = ();
    fn create(input: &Self::Input, context: &ComponentContext<Self>) -> Self {
        if std::env::var_os("PROBE_FONTS").is_some() {
            fonts::install_winui_font().expect("install fonts");
        }
        let delay = input.0;
        Self {
            keeper: input.1 == "Lifecycle keeper",
            rounds: 0,
            title: input.1,
            _task: context.spawn_background(move |_| {
                std::thread::sleep(Duration::from_millis(delay));
            }),
        }
    }
    fn update(&mut self, _: (), context: &ComponentContext<Self>) {
        if self.keeper && self.rounds < 3 {
            self.rounds += 1;
            println!(
                "Creating new control window {} under the existing host",
                self.rounds
            );
            assert!(context.open_window(View::component::<Probe>((400, "Lifecycle new child"))));
            self._task =
                context.spawn_background(move |_| std::thread::sleep(Duration::from_millis(1000)));
            return;
        }
        if std::env::var_os("PROBE_NATIVE_CLOSE").is_some() {
            use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
            use windows::Win32::UI::WindowsAndMessaging::*;
            unsafe extern "system" fn close(hwnd: HWND, data: LPARAM) -> windows::core::BOOL {
                let expected = *(data.0 as *const &'static str);
                let mut title = [0u16; 128];
                let length = GetWindowTextW(hwnd, &mut title);
                if String::from_utf16_lossy(&title[..length as usize]) == expected {
                    PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)).unwrap();
                }
                true.into()
            }
            unsafe {
                EnumThreadWindows(
                    windows::Win32::System::Threading::GetCurrentThreadId(),
                    Some(close),
                    LPARAM(&self.title as *const _ as isize),
                )
                .unwrap();
            }
        } else {
            assert!(context.window().request_close());
        }
    }
    fn view(&self, input: &Self::Input, context: &mut ViewContext<Self>) -> View {
        context.window_title(input.1);
        if std::env::var_os("PROBE_MICA").is_some() {
            context.window_visuals(WindowVisuals::new().backdrop(WindowBackdrop::Mica));
        }
        let text = TextBlock::new().text("Lifecycle diagnostic; no accounts or data are loaded.");
        if self.keeper {
            return text.into();
        }
        match std::env::var("PROBE_CONTROLS").as_deref() {
            Ok("toggle") => ToggleSwitch::new().is_on(false).into(),
            Ok("combo") => ComboBox::new().items_source(["A", "B"]).into(),
            Ok("password") => PasswordBox::new().into(),
            Ok("button") => Button::new().content("Diagnostic").into(),
            Ok("nav") => NavigationView::new()
                .pane_title("Diagnostic")
                .slots([
                    SlotView::collection(
                        NavigationViewSlot::MenuItems,
                        [
                            (
                                "general",
                                NavigationViewItem::new().slots([SlotView::new(
                                    NavigationViewItemSlot::Content,
                                    TextBlock::new().text("General"),
                                )]),
                            ),
                            (
                                "accounts",
                                NavigationViewItem::new().slots([SlotView::new(
                                    NavigationViewItemSlot::Content,
                                    TextBlock::new().text("Accounts"),
                                )]),
                            ),
                        ],
                    ),
                    SlotView::new(NavigationViewSlot::Content, text),
                ])
                .into(),
            Ok("all") => NavigationView::new()
                .pane_title("Diagnostic")
                .slots([
                    SlotView::collection(
                        NavigationViewSlot::MenuItems,
                        [
                            (
                                "general",
                                NavigationViewItem::new().slots([SlotView::new(
                                    NavigationViewItemSlot::Content,
                                    TextBlock::new().text("General"),
                                )]),
                            ),
                            (
                                "accounts",
                                NavigationViewItem::new().slots([SlotView::new(
                                    NavigationViewItemSlot::Content,
                                    TextBlock::new().text("Accounts"),
                                )]),
                            ),
                        ],
                    ),
                    SlotView::new(
                        NavigationViewSlot::Content,
                        StackPanel::new().children((
                            text,
                            ToggleSwitch::new().is_on(false),
                            ComboBox::new().items_source(["A", "B"]),
                            PasswordBox::new(),
                            Button::new().content("Button"),
                        )),
                    ),
                ])
                .into(),
            _ => text.into(),
        }
    }
}
fn main() {
    let mode = std::env::args().nth(1).expect("multi or restart");
    if mode == "replace" {
        println!("ONE host, THREE control windows created and closed in sequence");
        App::run_component::<Probe>((1000, "Lifecycle keeper"))
            .expect("new windows under existing host");
        println!("PASS: new control windows were recreated without restarting the host");
    } else if mode == "multi" {
        println!("ONE Application::Start, TWO newly created windows");
        App::run_windows([
            View::component::<Probe>((1000, "Lifecycle probe A")),
            View::component::<Probe>((2000, "Lifecycle probe B")),
        ])
        .expect("same-host windows");
        println!("PASS: both windows were created and destroyed under one host");
    } else {
        for attempt in 1..=2 {
            println!("Application::Start attempt {attempt}");
            let result = App::run_component::<Probe>((1000, "Lifecycle host restart probe"));
            println!("Application::Start attempt {attempt} returned: {result:?}");
            result.expect("host start");
        }
        println!("PASS: host restart");
    }
}
