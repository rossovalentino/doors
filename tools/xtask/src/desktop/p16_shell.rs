//! P16 SHELL-A: an ordinary Windows console program (`shellprobe.exe`,
//! built with the pinned MSVC toolchain and SDK import libraries, no C
//! runtime) runs from Terminal and uses shell32.dll and CreateProcess:
//! CommandLineToArgvW, known folders and item ID lists,
//! SHCreateDirectoryEx, SHFileOperation, SHGetFileInfo, ShellExecuteEx and
//! CreateProcess starting the probe itself as a child whose exit code comes
//! back (once with a command line longer than the startup record), icons
//! (user32, the system image list), SHBrowseForFolder, CreateProcess with
//! CREATE_SUSPENDED and its own environment block (ResumeThread starts it)
//! and CreateThread CREATE_SUSPENDED with Suspend/ResumeThread counts. Then
//! `shellprobe.exe tray` adds a notification icon: the taskbar's "Show
//! hidden icons" chevron opens the flyout and a click on the icon reaches
//! the program as NIN_SELECT.
use super::p15_connect::{Probe, build, run_probe, stage};
use super::*;

const SHELLPROBE: Probe = Probe {
    name: "shellprobe",
    directory: "tests/fixtures/windows-shell",
    libraries: &[
        "kernel32.lib",
        "shell32.lib",
        "Ole32.Lib",
        "User32.Lib",
        "Gdi32.Lib",
        "ComCtl32.Lib",
    ],
    modules: &[
        "comctl32.dll",
        "gdi32.dll",
        "kernel32.dll",
        "ole32.dll",
        "shell32.dll",
        "user32.dll",
    ],
};
const RESOLUTION: &str = "1280x720";
/// Steps shellprobe reports, in order.
const STEPS: [&str; 15] = [
    "argv",
    "known-folders",
    "item-ids",
    "create-directory",
    "file-operation",
    "file-info",
    "shell-execute",
    "create-process",
    "long-command-line",
    "icons",
    "browse-folder",
    "suspended-process",
    "suspended-thread",
    "pipe",
    "streams",
];

pub(super) fn verify(run: &mut Run, image: &str) -> Result<(), String> {
    let target = run.generated_directory(&format!("out/builds/{}/shell-a", run.id))?;
    let fixture = build(run, &target, &SHELLPROBE)?;
    let staged = stage(
        run,
        image,
        std::slice::from_ref(&fixture),
        &target,
        "desktop-shell",
        &[
            "windows/shell32.dll",
            "windows/ole32.dll",
            "windows/user32.dll",
            "windows/gdi32.dll",
            "windows/comctl32.dll",
            "windows/ntdll.dll",
            "windows/kernel32.dll",
        ],
    )?;
    let disk = super::win32::fresh_disk(run, "shell-a")?;
    let mut record = session::launch_desktop_accelerated(run, &staged, &disk, false, RESOLUTION)?;
    let result: Result<Value, String> = (|| {
        let mut c = connection(&record)?;
        thread::sleep(Duration::from_secs(3));
        super::win32::open_terminal(run, &record, &mut c)?;
        thread::sleep(Duration::from_secs(1));
        let probe = run_probe(
            run,
            &record,
            &mut c,
            &fixture,
            "shellprobe.exe",
            "SHELL-A",
            &STEPS,
        )?;
        let frame = scene::capture_settled(run, &mut c, "shell-a-terminal", &[])?;
        let shot = super::win32::keep(run, &frame, "shell-a-terminal")?;
        let tray = tray(run, &record, &mut c)?;
        // FOF_ALLOWUNDO: a delete goes to the user's Recycle Bin (guest
        // only, as the reference PC's bin is the owner's).
        let before = session::transcript(&run.root, &record)?.len();
        let offset = serial(&run.root, &record)?.len();
        super::type_line(&mut c, "shellprobe.exe recycle")?;
        let recycle =
            super::p15_connect::console_after(run, &record, before, "SHELL-A recycle PASSED", 30)?;
        // The shell's prompt is back before the next line is typed (a key
        // typed while the program is ending is lost).
        super::win32::wait_after(run, &record, offset, "DOOR_P16 DSH IDLE", 15)?;
        thread::sleep(Duration::from_millis(500));
        let downloaded = downloaded(run, &record, &mut c)?;
        let open_with = open_with(run, &record, &mut c)?;
        Ok(
            json!({"probe":probe,"screenshot":shot,"tray":tray,"recycle":recycle,"downloaded":downloaded,"open_with":open_with}),
        )
    })();
    let details = super::win32::quit_on_error(run, &record, result, "shell-a-failure")?;
    let mut c = connection(&record)?;
    super::win32::finish(run, &mut record, &mut c)?;
    run.check("p16-shell-a", json!({"fixture":fixture,"image":staged,"run":details,"session":record,
        "scope":"An MSVC-built Windows console program using shell32.dll (CommandLineToArgvW, known folders, item ID lists, SHCreateDirectoryEx, SHFileOperation, SHGetFileInfo, ShellExecuteEx, icons, the system image list, SHBrowseForFolder, Shell_NotifyIcon through the hidden-icons flyout, SHFileOperation FOF_ALLOWUNDO into the Recycle Bin with SHQueryRecycleBin) and kernel32's CreateProcess with a long command line, CREATE_SUSPENDED and lpEnvironment, and CreateThread CREATE_SUSPENDED with SuspendThread/ResumeThread in the guest; anonymous pipes as a child's standard handles (STARTF_USESTDHANDLES); alternate data streams (a Zone.Identifier stream written, read, listed by FindFirstStreamW, carried by CopyFile, deleted alone); the mark of the web: File Explorer's Open File - Security Warning for a downloaded program, unblocked by clearing Always ask, then run without asking; Open with: ShellExecuteEx of a file of an unregistered type (ERROR_NO_ASSOCIATION without UI) opens File Explorer's app picker, which opens it in Notes and keeps that choice, and the openas verb asks again"}));
    Ok(())
}
/// The mark of the web: `shellprobe.exe mark` saves a copy as
/// Downloads\downloaded.exe with a Zone.Identifier stream (zone 3).
/// Opening it from File Explorer's address bar shows "Open File - Security
/// Warning"; clearing "Always ask before opening this file" (Alt+A) and Run
/// (Alt+R) removes the mark and runs it (it exits 15); opening it again
/// runs it without asking.
fn downloaded(run: &mut Run, record: &Value, c: &mut qmp::Client) -> Result<Value, String> {
    const NATIVE: &str = "/users/1001/Downloads/downloaded.exe";
    let before = session::transcript(&run.root, record)?.len();
    super::type_line(c, "shellprobe.exe mark")?;
    let marked = super::p15_connect::console_after(run, record, before, "SHELL-A mark PASSED", 30)?;
    let tail = |run: &Run, from: usize| -> Result<String, String> {
        let text = serial(&run.root, record)?;
        Ok(text.get(from.min(text.len())..).unwrap_or("").to_owned())
    };
    let wait = |run: &Run, from: usize, needle: &str| -> Result<String, String> {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(line) = tail(run, from)?.lines().find(|l| l.contains(needle)) {
                return Ok(line.to_owned());
            }
            if Instant::now() > deadline {
                return Err(format!("{needle} missing"));
            }
            thread::sleep(Duration::from_millis(100));
        }
    };
    // File Explorer, its address bar, the downloaded program's path.
    let before = serial(&run.root, record)?.len();
    key(c, &["meta_l", "e"])?;
    wait(run, before, "DOOR_P12 APP_LAUNCHED")?;
    thread::sleep(Duration::from_secs(3));
    let open = |c: &mut qmp::Client| -> Result<(), String> {
        key(c, &["ctrl", "l"])?;
        thread::sleep(Duration::from_millis(400));
        super::type_line(c, "C:\\Users\\1001\\Downloads\\downloaded.exe")
    };
    let before = serial(&run.root, record)?.len();
    open(c)?;
    let warning = wait(
        run,
        before,
        &format!("DOOR_P16 FILES SECURITY WARNING path={NATIVE}"),
    )?;
    if !warning.contains("zone=3") || !warning.contains("publisher=Unknown") {
        return Err(format!("unexpected security warning: {warning}"));
    }
    thread::sleep(Duration::from_millis(800));
    let frame = scene::capture_settled(run, c, "shell-a-security-warning", &[])?;
    let shot = super::win32::keep(run, &frame, "shell-a-security-warning")?;
    key(c, &["alt", "a"])?;
    thread::sleep(Duration::from_millis(300));
    let before = serial(&run.root, record)?.len();
    key(c, &["alt", "r"])?;
    let ran = wait(
        run,
        before,
        &format!("DOOR_P16 FILES SECURITY RUN path={NATIVE} unblocked=1"),
    )?;
    let exit = wait(run, before, "exit=15")?;
    // Unblocked: the next open runs it at once.
    thread::sleep(Duration::from_secs(1));
    let before = serial(&run.root, record)?.len();
    open(c)?;
    let again = wait(run, before, &format!("DOOR_P12 FILES LAUNCH path={NATIVE}"))?;
    wait(run, before, "exit=15")?;
    if tail(run, before)?.contains("DOOR_P16 FILES SECURITY WARNING") {
        return Err("the unblocked program was warned about again".into());
    }
    key(c, &["alt", "f4"])?;
    thread::sleep(Duration::from_secs(1));
    Ok(
        json!({"marked":marked,"warning":warning,"screenshot":shot,"ran":ran,"exit":exit,"again":again}),
    )
}
/// Open with (guest only: on the reference PC it would open Windows'
/// picker): `shellprobe.exe openwith` ShellExecuteEx-es a file of a type no
/// app is registered for, after checking that without UI that is
/// ERROR_NO_ASSOCIATION. File Explorer takes it, as OpenWith.exe does: its
/// folder and the picker ("Always" starts checked for a type with no app);
/// Enter opens it in Notes and keeps Notes for the type. `shellprobe.exe
/// openas` asks again although the type has an app now; Escape cancels.
fn open_with(run: &mut Run, record: &Value, c: &mut qmp::Client) -> Result<Value, String> {
    const FILE: &str = "doors-open-with.doorsopen";
    let wait = |run: &Run, from: usize, needle: &str| -> Result<String, String> {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let text = serial(&run.root, record)?;
            if let Some(line) = text
                .get(from.min(text.len())..)
                .unwrap_or("")
                .lines()
                .find(|l| l.contains(needle))
            {
                return Ok(line.to_owned());
            }
            if Instant::now() > deadline {
                return Err(format!("{needle} missing"));
            }
            thread::sleep(Duration::from_millis(100));
        }
    };
    // Terminal has the keyboard (the pointer rests where the tray step
    // left it, over Terminal).
    let mut pointer = (640, 300);
    super::click(c, &mut pointer, 640, 300)?;
    thread::sleep(Duration::from_millis(600));
    let before = session::transcript(&run.root, record)?.len();
    let offset = serial(&run.root, record)?.len();
    super::type_line(c, "shellprobe.exe openwith")?;
    let quiet =
        super::p15_connect::console_after(run, record, before, "SHELL-A openwith quiet", 30)?;
    let asked =
        super::p15_connect::console_after(run, record, before, "SHELL-A openwith asked", 30)?;
    let handed = wait(run, offset, "DOOR_P16 OPEN_WITH W ")?;
    if !handed.ends_with(FILE) {
        return Err(format!("Open with got {handed}"));
    }
    let picker = wait(run, offset, "DOOR_P14 FILES OPEN_WITH ext=.doorsopen")?;
    thread::sleep(Duration::from_millis(1500));
    let frame = scene::capture_settled(run, c, "shell-a-open-with", &[])?;
    let picker_shot = super::win32::keep(run, &frame, "shell-a-open-with")?;
    // Enter: the first app (Notes), "Always use this app" checked.
    let offset = serial(&run.root, record)?.len();
    key(c, &["ret"])?;
    let saved = wait(
        run,
        offset,
        "DOOR_P14 FILES DEFAULT_APP ext=.doorsopen app=",
    )?;
    if !saved.ends_with("saved=1") {
        return Err(format!("the choice was not kept: {saved}"));
    }
    let opened = wait(run, offset, "DOOR_P14 FILES OPEN app=")?;
    if !opened.ends_with(FILE) {
        return Err(format!("opened {opened}"));
    }
    thread::sleep(Duration::from_secs(2));
    let frame = scene::capture_settled(run, c, "shell-a-open-with-notes", &[])?;
    let notes_shot = super::win32::keep(run, &frame, "shell-a-open-with-notes")?;
    // Close Notes and File Explorer; Terminal again.
    key(c, &["alt", "f4"])?;
    thread::sleep(Duration::from_secs(1));
    key(c, &["alt", "f4"])?;
    thread::sleep(Duration::from_secs(1));
    super::click(c, &mut pointer, 640, 300)?;
    thread::sleep(Duration::from_millis(600));
    // openas: the picker although .doorsopen has an app now.
    let before = session::transcript(&run.root, record)?.len();
    let offset = serial(&run.root, record)?.len();
    super::type_line(c, "shellprobe.exe openas")?;
    let asked_again =
        super::p15_connect::console_after(run, record, before, "SHELL-A openas asked", 30)?;
    let chosen = wait(run, offset, "DOOR_P16 OPEN_WITH A ")?;
    let picker_again = wait(run, offset, "DOOR_P14 FILES OPEN_WITH ext=.doorsopen")?;
    thread::sleep(Duration::from_millis(1500));
    let frame = scene::capture_settled(run, c, "shell-a-open-as", &[])?;
    let open_as_shot = super::win32::keep(run, &frame, "shell-a-open-as")?;
    let offset = serial(&run.root, record)?.len();
    key(c, &["esc"])?;
    thread::sleep(Duration::from_millis(800));
    if serial(&run.root, record)?
        .get(offset..)
        .unwrap_or("")
        .contains("DOOR_P14 FILES OPEN app=")
    {
        return Err("Escape opened the file".into());
    }
    key(c, &["alt", "f4"])?;
    thread::sleep(Duration::from_secs(1));
    Ok(
        json!({"quiet":quiet,"asked":asked,"handed":handed,"picker":picker,"picker_screenshot":picker_shot,
        "saved":saved,"opened":opened,"notes_screenshot":notes_shot,"asked_again":asked_again,
        "chosen":chosen,"picker_again":picker_again,"open_as_screenshot":open_as_shot}),
    )
}
/// `shellprobe.exe tray`: its notification icon is in the hidden-icons
/// flyout; the chevron opens it and a click on the icon reaches the
/// program (NIN_SELECT), which then exits.
fn tray(run: &mut Run, record: &Value, c: &mut qmp::Client) -> Result<Value, String> {
    use door_window::notify_icons as ni;
    let before = session::transcript(&run.root, record)?.len();
    super::type_line(c, "shellprobe.exe tray")?;
    super::p15_connect::console_after(run, record, before, "SHELL-A tray added", 60)?;
    thread::sleep(Duration::from_secs(1));
    let layout = door_window::taskbar::Layout::new(1280, 720);
    let centre = |r: door_window::notify_icons::Rect| {
        (r.x() + r.width() as i32 / 2, r.y() + r.height() as i32 / 2)
    };
    // Terminal took the keyboard; the pointer has not moved since boot.
    let mut pointer = (640, 360);
    let (x, y) = centre(ni::chevron(layout, false));
    super::click(c, &mut pointer, x, y)?;
    thread::sleep(Duration::from_millis(800));
    let frame = scene::capture_settled(run, c, "shell-a-tray-flyout", &[])?;
    let flyout = super::win32::keep(run, &frame, "shell-a-tray-flyout")?;
    let (x, y) = centre(ni::cell(ni::flyout(layout, false, 1), 0));
    super::click(c, &mut pointer, x, y)?;
    let output = super::p15_connect::console_after(run, record, before, "SHELL-A tray PASSED", 30)?;
    // The flyout kept the keyboard: close it and give Terminal the focus
    // back (its window fills the middle of the screen) before typing.
    super::key(c, &["esc"])?;
    thread::sleep(Duration::from_millis(400));
    super::click(c, &mut pointer, 640, 300)?;
    thread::sleep(Duration::from_secs(1));
    Ok(json!({"flyout":flyout,"output":output}))
}
