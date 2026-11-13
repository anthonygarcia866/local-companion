// `glim.exe --run <name>` (local agency v1, part 2): for Claude Code, Muse
// or a terminal. Runs an APPROVED, unchanged runbook in this process (the
// running Glim is not involved) and prints each step's OK/FAIL with its
// short English detail, never a value or a command's output.
//
// Exit codes: 0 every step OK; 1 a step failed; 2 no such runbook, a bad
// file or no name; 3 not approved, or changed since it was approved;
// 4 a step asks every run (ask = true): start it from Glim instead.
//
// Approving is never possible from here: only Settings → Runbooks can.

use super::{run_now, Outcome, Refusal, Trigger};

pub const FLAG: &str = "--run";

/// `Some(name)` for `--run <name>`, `Some(None)` for a bare `--run`.
pub fn requested(args: &[String]) -> Option<Option<String>> {
    let at = args.iter().position(|a| a == FLAG)?;
    Some(args.get(at + 1).filter(|n| !n.starts_with("--")).cloned())
}

/// The exit code for a run's result.
pub fn exit_code(r: &Result<Vec<Outcome>, Refusal>) -> i32 {
    match r {
        Ok(steps) if steps.iter().all(|o| o.ok) => 0,
        Ok(_) => 1,
        Err(Refusal::NotFound(_) | Refusal::Invalid(_)) => 2,
        Err(Refusal::NotApproved | Refusal::Changed) => 3,
        Err(Refusal::Asks(_)) => 4,
    }
}

/// The line printed for a step.
pub fn step_line(o: &Outcome) -> String {
    format!("step {}/{} {} {}: {}", o.step, o.total, o.kind, if o.ok { "OK" } else { "FAIL" }, o.detail)
}

/// A release glim.exe is a windowed app: print into the terminal that
/// started it, when it didn't hand over its own pipes.
fn attach_console() {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::System::Console::{AttachConsole, GetStdHandle, ATTACH_PARENT_PROCESS, STD_OUTPUT_HANDLE};
        let has_out = GetStdHandle(STD_OUTPUT_HANDLE).is_ok_and(|h| !h.is_invalid() && !h.0.is_null());
        if !has_out {
            let _ = AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}

pub fn main(name: Option<String>) -> i32 {
    use std::io::Write;
    attach_console();
    let mut out = std::io::stdout();
    let Some(name) = name else {
        let _ = writeln!(out, "usage: glim.exe --run <runbook name>   (runbooks live in {})", super::show(&super::dir()));
        return 2;
    };
    let _ = writeln!(out, "Glim runbook {name}");
    let result = run_now(&name, None, Trigger::Cli, &[], &mut |o| {
        let _ = writeln!(std::io::stdout(), "{}", step_line(o));
    });
    let code = exit_code(&result);
    let _ = match &result {
        Ok(steps) => {
            let done = steps.iter().filter(|o| o.ok).count();
            let total = steps.first().map(|o| o.total).unwrap_or(0);
            writeln!(out, "{} ({done}/{total} steps) exit {code}", if code == 0 { "OK" } else { "FAIL" })
        }
        Err(r) => writeln!(out, "REFUSED: {} exit {code}", r.message()),
    };
    let _ = out.flush();
    code
}
