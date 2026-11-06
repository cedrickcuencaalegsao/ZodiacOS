use crate::kprintln;
use crate::process::services::{self, State};
use crate::security::service as sec;
use alloc::vec::Vec;

fn is_root() -> bool {
    sec::whoami().map_or(false, |s| s.is_root())
}

pub fn run(args: &[&str]) {
    match args.first().copied() {
        Some("list") => {
            // built-ins run inside the boot "kernel" task (scheduler slot 0)
            let kpid = crate::scheduler::with(|s| s.tasks[0].pid);
            kprintln!("{:<16} {:<8} {:<6} {}", "NAME", "STATE", "PID", "INFO");
            for b in services::BUILTINS {
                kprintln!(
                    "{:<16} {:<8} {:<6} {}",
                    b.name,
                    "builtin",
                    kpid,
                    (b.status)()
                );
            }
            for s in services::list() {
                let st = if s.state == State::Running {
                    "running"
                } else {
                    "stopped"
                };
                match s.pid {
                    Some(p) => {
                        kprintln!("{:<16} {:<8} {:<6} restarts: {}", s.name, st, p, s.restarts)
                    }
                    None => kprintln!(
                        "{:<16} {:<8} {:<6} restarts: {}",
                        s.name,
                        st,
                        "-",
                        s.restarts
                    ),
                }
            }
        }
        Some(cmd @ ("start" | "kill" | "restart" | "remove")) => {
            if !is_root() {
                kprintln!("service: permission denied (root only)");
                return;
            }
            let Some(name) = args.get(1) else {
                kprintln!("usage: service {} <name>", cmd);
                return;
            };
            if services::is_builtin(name) {
                kprintln!("service: '{}' is a built-in, not a task (read-only)", name);
                return;
            }
            let r = match cmd {
                "start" => services::start(name),
                "kill" => services::kill(name),
                "restart" => services::restart(name),
                _ => services::remove(name),
            };
            match r {
                Ok(()) => kprintln!("service '{}': {} ok", name, cmd),
                Err(e) => kprintln!("service: {:?}", e),
            }
        }
        Some("new") => {
            if !is_root() {
                kprintln!("service: permission denied (root only)");
                return;
            }
            let (Some(name), Some(tpl)) = (args.get(1), args.get(2)) else {
                let names: Vec<&str> = services::TEMPLATES.iter().map(|t| t.0).collect();
                kprintln!("usage: service new <name> <template>");
                kprintln!("templates: {}", names.join(", "));
                return;
            };
            match services::new_from_template(name, tpl) {
                Ok(()) => kprintln!(
                    "service '{}' created (stopped). Run: service start {}",
                    name,
                    name
                ),
                Err(e) => kprintln!("service: {:?}", e),
            }
        }
        _ => kprintln!("usage: service <list|new|start|kill|restart|remove>"),
    }
}
