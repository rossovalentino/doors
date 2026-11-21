//! P16 ACCOUNTS-A: a second local account and a standard user's elevation.
//! First boot (the owner, an administrator, signed in with the test PIN):
//! Settings > Accounts > Other users > Add account creates "Guest", a
//! standard user with a password. Second boot on the same private disk:
//! the sign-in screen lists both accounts at the bottom left; the Guest
//! tile is clicked and Guest's password typed, so the session is uid 1002.
//! In Guest's session Win+R `regedit` asks User Account Control, which for
//! a standard user is the credential prompt: the owner's PIN typed there is
//! checked by the session service before Registry Editor opens. In Guest's
//! Terminal, `secprobe.exe standard` checks the standard user's token and
//! advapi32's security functions with it (SEC-S, see p16_security).
use super::p15_connect::{build, run_probe, stage};
use super::*;

const RESOLUTION: (u32, u32) = (1280, 720);
const NAME: &str = "Guest";
const PASSWORD: &str = "guest2468";

fn wait_for(run: &Run, record: &Value, before: usize, needle: &str) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(40);
    loop {
        let text = serial(&run.root, record)?;
        if let Some(line) = text
            .get(before.min(text.len())..)
            .unwrap_or("")
            .lines()
            .find(|l| l.contains(needle))
        {
            return Ok(line.to_owned());
        }
        if text.contains("KERNEL_PANIC") || text.contains("APP FAILED") {
            return Err("guest failed; inspect serial".into());
        }
        if Instant::now() >= deadline {
            return Err(format!("{needle} missing"));
        }
        thread::sleep(Duration::from_millis(100));
    }
}
/// Settings page id (`Page as usize`, what Settings reports).
fn page(p: door_workspace::settings_layout::Page) -> String {
    format!("DOOR_P12 SETTINGS PAGE id={}", p as usize)
}

pub(super) fn verify(run: &mut Run, image: &str) -> Result<(), String> {
    use door_workspace::settings_layout::Page;
    let target = run.generated_directory(&format!("out/builds/{}/accounts-a", run.id))?;
    let probe = build(run, &target, &super::p16_security::SECPROBE)?;
    let staged = stage(
        run,
        image,
        std::slice::from_ref(&probe),
        &target,
        "desktop-accounts",
        &[
            "windows/advapi32.dll",
            "windows/ntdll.dll",
            "windows/kernel32.dll",
        ],
    )?;
    let image = staged.as_str();
    let disk = super::win32::fresh_disk(run, "accounts-a")?;
    let resolution = format!("{}x{}", RESOLUTION.0, RESOLUTION.1);
    // Boot 1: the owner adds Guest.
    let mut record = session::launch_desktop_accelerated(run, image, &disk, false, &resolution)?;
    let added: Result<Value, String> = (|| {
        let mut c = connection(&record)?;
        thread::sleep(Duration::from_secs(2));
        let before = serial(&run.root, &record)?.len();
        key(&mut c, &["meta_l", "r"])?;
        thread::sleep(Duration::from_millis(400));
        type_line(&mut c, "ms-settings:otherusers")?;
        wait_for(run, &record, before, &page(Page::OtherUsers))?;
        thread::sleep(Duration::from_millis(1500));
        // Tab to Add account, Return opens "Create a user for this PC".
        key(&mut c, &["tab"])?;
        key(&mut c, &["ret"])?;
        thread::sleep(Duration::from_millis(600));
        for ch in NAME.chars() {
            super::administration::type_character(&mut c, ch)?;
        }
        key(&mut c, &["tab"])?;
        for ch in PASSWORD.chars() {
            super::administration::type_character(&mut c, ch)?;
        }
        let before = serial(&run.root, &record)?.len();
        key(&mut c, &["ret"])?;
        let created = wait_for(run, &record, before, "DOOR_P16 ACCOUNT created standard=1")?;
        wait_for(run, &record, before, &format!("Account added: {NAME}"))?;
        thread::sleep(Duration::from_millis(800));
        scene::capture_settled(run, &mut c, "p16-accounts-added", &[])?;
        key(&mut c, &["alt", "f4"])?;
        thread::sleep(Duration::from_secs(1));
        start_shut_down(&mut c)?;
        wait_serial(
            &run.root,
            &record,
            "DOOR_P08 CLEANUP PASS",
            Duration::from_secs(40),
        )?;
        Ok(json!({"created":created}))
    })();
    let added = super::win32::quit_on_error(run, &record, added, "p16-accounts-add-failure")?;
    seal_serial(&run.root, &mut record)?;
    // Boot 2: Guest's tile and password at the sign-in screen.
    let tile = door_window::login::GateLayout::new(
        RESOLUTION.0,
        RESOLUTION.1,
        door_window::login::Page::SignIn,
    )
    .users[1];
    session::sign_in_as(Some(session::SignInAs {
        tile: (tile.x() + 30, tile.y() + tile.height() as i32 / 2),
        start: (RESOLUTION.0 as i32 / 2, RESOLUTION.1 as i32 / 2),
        password: PASSWORD.into(),
    }));
    let launched = session::launch_desktop_accelerated(run, image, &disk, false, &resolution);
    session::sign_in_as(None);
    let mut guest = launched?;
    let elevated: Result<Value, String> = (|| {
        let text = serial(&run.root, &guest)?;
        let signin = text
            .lines()
            .find(|l| l.contains("DOOR_P16 SIGNIN account="))
            .map(str::to_owned)
            .ok_or("no account sign-in line")?;
        if !signin.ends_with("account=1002") {
            return Err(format!("signed in as the wrong account: {signin}"));
        }
        wait_for(run, &guest, 0, "SESSION_READY uid=1002")?;
        let mut c = connection(&guest)?;
        thread::sleep(Duration::from_secs(2));
        // A standard user: Registry Editor asks for an administrator's
        // credentials; the owner's PIN approves.
        let before = serial(&run.root, &guest)?.len();
        key(&mut c, &["meta_l", "r"])?;
        thread::sleep(Duration::from_millis(400));
        type_line(&mut c, "regedit")?;
        wait_for(run, &guest, before, "DOOR_P16 UAC PROMPT")?;
        thread::sleep(Duration::from_millis(800));
        scene::capture_settled(run, &mut c, "p16-accounts-credential-prompt", &[])?;
        // A wrong password first: the prompt stays and says so.
        for ch in "1357".chars() {
            super::administration::type_character(&mut c, ch)?;
        }
        let before = serial(&run.root, &guest)?.len();
        key(&mut c, &["ret"])?;
        wait_for(run, &guest, before, "DOOR_P16 UAC CREDENTIAL rejected")?;
        for ch in super::p14_security::TEST_PIN.chars() {
            super::administration::type_character(&mut c, ch)?;
        }
        let before = serial(&run.root, &guest)?.len();
        key(&mut c, &["ret"])?;
        wait_for(run, &guest, before, "DOOR_P16 UAC CREDENTIAL accepted")?;
        wait_for(run, &guest, before, "DOOR_P16 UAC ANSWER yes")?;
        let opened = wait_for(run, &guest, before, &page(Page::Registry))?;
        thread::sleep(Duration::from_millis(800));
        scene::capture_settled(run, &mut c, "p16-accounts-elevated", &[])?;
        key(&mut c, &["alt", "f4"])?;
        thread::sleep(Duration::from_secs(1));
        // The standard user's token and advapi32's security functions.
        super::win32::open_terminal(run, &guest, &mut c)?;
        thread::sleep(Duration::from_secs(2));
        let security = run_probe(
            run,
            &guest,
            &mut c,
            &probe,
            "secprobe.exe standard guest",
            "SEC-S",
            &super::p16_security::STEPS,
        )?;
        scene::capture_settled(run, &mut c, "p16-accounts-standard-security", &[])?;
        let before = serial(&run.root, &guest)?.len();
        key(&mut c, &["alt", "f4"])?;
        super::win32::wait_after(run, &guest, before, "CLIENT_EXIT", 15)?;
        thread::sleep(Duration::from_millis(800));
        // Sign out (Win+X > Shut down or sign out > Sign out): the session
        // ends and the sign-in screen comes back on the same boot, where
        // the owner signs in again.
        let before = serial(&run.root, &guest)?.len();
        key(&mut c, &["meta_l", "x"])?;
        thread::sleep(Duration::from_millis(600));
        key(&mut c, &["up"])?;
        key(&mut c, &["up"])?;
        key(&mut c, &["right"])?;
        thread::sleep(Duration::from_millis(300));
        key(&mut c, &["ret"])?;
        wait_for(run, &guest, before, "DOOR_P16 SIGNOUT requested")?;
        let signed_out = wait_for(run, &guest, before, "DOOR_P16 SIGNOUT uid=1002")?;
        wait_for(run, &guest, before, "DOOR_P12 SIGNIN AUTHORITY READY")?;
        wait_for(run, &guest, before, "DOOR_P14 SIGNIN surface=signin")?;
        thread::sleep(Duration::from_secs(1));
        scene::capture_settled(run, &mut c, "p16-accounts-signed-out", &[])?;
        let before = serial(&run.root, &guest)?.len();
        for ch in super::p14_security::TEST_PIN.chars() {
            super::administration::type_character(&mut c, ch)?;
        }
        key(&mut c, &["ret"])?;
        let owner = wait_for(run, &guest, before, "SESSION_READY uid=1001")?;
        wait_for(run, &guest, before, "DOOR_P11 READY")?;
        thread::sleep(Duration::from_secs(2));
        scene::capture_settled(run, &mut c, "p16-accounts-owner-again", &[])?;
        start_shut_down(&mut c)?;
        wait_serial(
            &run.root,
            &guest,
            "DOOR_P08 CLEANUP PASS",
            Duration::from_secs(40),
        )?;
        Ok(
            json!({"signin":signin,"registry":opened,"security":security,"signed_out":signed_out,"owner_again":owner}),
        )
    })();
    let elevated =
        super::win32::quit_on_error(run, &guest, elevated, "p16-accounts-guest-failure")?;
    seal_serial(&run.root, &mut guest)?;
    run.check(
        "p16-accounts-a",
        json!({"owner_session":record,"guest_session":guest,"added":added,"guest":elevated,"fixtures":[probe],"image":staged,
            "scope":"ACCOUNTS-A: Settings > Other users adds a standard local account; the next boot's sign-in screen lists both accounts and signs in the second one (uid 1002) with its password; that standard user's User Account Control prompt asks for an administrator's credentials (a wrong one is refused) before Registry Editor opens; signing out (Win+X > Shut down or sign out > Sign out) ends that session and brings back the sign-in screen on the same boot, where the owner signs in again; in the standard user's Terminal secprobe checks the standard token (elevation type Default, no Administrators group, no linked token) and advapi32's security functions with it (SEC-S)"}),
    );
    Ok(())
}
