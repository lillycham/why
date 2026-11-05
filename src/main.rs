//! `why <target>`: explain why a file, command or listening port is here.
//!
//! Targets:
//!   why <path>     symlink chain, Nix store roots, git history
//!   why <command>  the same, for the binary found on PATH
//!   why :<port>    the listening process and the chain that started it

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::{env, fs};

const NIX_STORE: &str = "/nix/store";
const MAX_CHAIN: usize = 16;

enum Target {
    Port(u16),
    Path(PathBuf),
}

/// Run a command and return its trimmed stdout, or `None` on any failure or empty output.
fn run(cmd: &str, args: &[&str], dir: Option<&Path>) -> Option<String> {
    let mut c = Command::new(cmd);
    c.args(args);
    if let Some(d) = dir {
        c.current_dir(d);
    }
    let out = c.output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim_end().to_string();
    (!s.is_empty()).then_some(s)
}

struct Out {
    color: bool,
}

impl Out {
    fn head(&self, s: &str) {
        if self.color {
            println!("\x1b[1m{s}\x1b[0m");
        } else {
            println!("{s}");
        }
    }

    fn line(&self, s: &str) {
        println!("  • {s}");
    }

    fn sub(&self, s: &str) {
        if self.color {
            println!("      \x1b[2m{s}\x1b[0m");
        } else {
            println!("      {s}");
        }
    }
}

fn classify(arg: &str) -> Result<Target, String> {
    if let Some(port) = arg.strip_prefix(':') {
        return port
            .parse()
            .map(Target::Port)
            .map_err(|_| format!("`{arg}` is not a valid port"));
    }
    let cwd = env::current_dir().map_err(|e| e.to_string())?;
    let direct = cwd.join(arg);
    if direct.symlink_metadata().is_ok() {
        return Ok(Target::Path(direct));
    }
    if !arg.contains('/') {
        if let Some(found) = find_in_path(arg) {
            return Ok(Target::Path(found));
        }
    }
    Err(format!(
        "`{arg}` is not a path, a command on PATH, or a :port"
    ))
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    env::split_paths(&env::var_os("PATH")?)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

// ---------------------------------------------------------------- paths

fn explain_path(o: &Out, start: &Path, requested: &str) {
    o.head(&format!("why {requested}"));

    // Follow the symlink chain one hop at a time so each hop is visible.
    let mut cur = start.to_path_buf();
    let mut hops = 0;
    while let Ok(target) = fs::read_link(&cur) {
        let next = match cur.parent() {
            Some(parent) => parent.join(&target),
            None => target,
        };
        o.line(&format!(
            "`{}` is a symlink to `{}`",
            cur.display(),
            next.display()
        ));
        cur = next;
        hops += 1;
        if hops >= MAX_CHAIN {
            o.line("symlink chain is too long; stopping");
            break;
        }
    }

    let real = match fs::canonicalize(&cur) {
        Ok(r) => r,
        Err(_) => {
            o.line(&format!(
                "`{}` does not exist (dangling symlink)",
                cur.display()
            ));
            return;
        }
    };

    if let Some(store_path) = nix_store_path(&real) {
        explain_nix(o, &store_path);
    }
    explain_git(o, &real);
}

/// `/nix/store/<hash>-<name>/bin/x` -> `/nix/store/<hash>-<name>`
fn nix_store_path(real: &Path) -> Option<PathBuf> {
    let rest = real.strip_prefix(NIX_STORE).ok()?;
    let first = rest.components().next()?;
    Some(Path::new(NIX_STORE).join(first))
}

fn explain_nix(o: &Out, sp: &Path) {
    let sp_s = sp.to_string_lossy();
    let name = sp
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.split_once('-'))
        .map_or("?", |(_, n)| n);
    o.line(&format!(
        "it lives in the Nix store, as the package `{name}`"
    ));

    if let Some(deriver) = run("nix-store", &["--query", "--deriver", &sp_s], None) {
        if !deriver.contains("unknown-deriver") {
            o.line("it was built from:");
            o.sub(&deriver);
        }
    }

    match run("nix-store", &["--query", "--roots", &sp_s], None) {
        Some(roots) => {
            let roots: Vec<&str> = roots
                .lines()
                .map(|l| l.split(" -> ").next().unwrap_or(l))
                .collect();
            o.line("it is kept alive by these GC roots:");
            for r in roots.iter().take(6) {
                o.sub(r);
            }
            if roots.len() > 6 {
                o.sub(&format!("... and {} more", roots.len() - 6));
            }
        }
        None => o.line("no GC root keeps it alive, so `nix-collect-garbage` will remove it"),
    }
}

struct Commit {
    hash: String,
    author: String,
    date: String,
    subject: String,
    body: String,
}

const LOG_FMT: &str = "--format=%h%x1f%an%x1f%as%x1f%s%x1f%b%x1e";

fn parse_commits(raw: &str) -> Vec<Commit> {
    raw.split('\x1e')
        .filter(|r| !r.trim().is_empty())
        .filter_map(|r| {
            let mut f = r.trim_start().splitn(5, '\x1f');
            Some(Commit {
                hash: f.next()?.to_string(),
                author: f.next()?.to_string(),
                date: f.next()?.to_string(),
                subject: f.next()?.to_string(),
                body: f.next()?.trim().to_string(),
            })
        })
        .collect()
}

fn show_commit(o: &Out, label: &str, c: &Commit) {
    o.line(&format!(
        "{label} in {} by {} on {}:",
        c.hash, c.author, c.date
    ));
    o.sub(&format!("\"{}\"", c.subject));
    let body: Vec<&str> = c.body.lines().collect();
    for l in body.iter().take(6) {
        o.sub(l);
    }
    if body.len() > 6 {
        o.sub("...");
    }
}

fn explain_git(o: &Out, real: &Path) {
    let dir = if real.is_dir() {
        real
    } else {
        real.parent().unwrap_or(real)
    };
    let Some(top) = run("git", &["rev-parse", "--show-toplevel"], Some(dir)) else {
        // Nix store paths have no repository; that is expected, so stay quiet.
        if nix_store_path(real).is_none() {
            o.line("it is not inside a git repository, so there is no history to read");
        }
        return;
    };
    let top = fs::canonicalize(&top).unwrap_or_else(|_| PathBuf::from(&top));
    let Ok(rel) = real.strip_prefix(&top) else {
        return;
    };
    let rel_s = if rel.as_os_str().is_empty() {
        ".".into()
    } else {
        rel.to_string_lossy()
    };
    let rel_s: &str = &rel_s;
    let repo = top.file_name().and_then(|n| n.to_str()).unwrap_or("?");

    let tracked = run("git", &["ls-files", "--", rel_s], Some(&top)).is_some();
    if !tracked {
        match run("git", &["check-ignore", "-v", "--", rel_s], Some(&top)) {
            Some(rule) => {
                o.line(&format!("git ignores it in `{repo}`, by this rule:"));
                o.sub(&rule);
            }
            None => o.line(&format!(
                "it is in `{repo}` but git does not track it (not committed yet)"
            )),
        }
        return;
    }

    let mut args = vec!["log", LOG_FMT];
    if real.is_file() {
        args.push("--follow");
    }
    args.extend(["--", rel_s]);
    let commits = run("git", &args, Some(&top))
        .map(|r| parse_commits(&r))
        .unwrap_or_default();
    let (Some(newest), Some(oldest)) = (commits.first(), commits.last()) else {
        o.line(&format!("it is staged in `{repo}` but has no commits yet"));
        return;
    };

    if commits.len() == 1 {
        show_commit(
            o,
            &format!("it was added to `{repo}`, and never changed since,"),
            oldest,
        );
        return;
    }
    show_commit(o, &format!("it was added to `{repo}`"), oldest);
    let n = commits.len() - 1;
    let times = if n == 1 { "time" } else { "times" };
    show_commit(o, &format!("it changed {n} {times}, most recently"), newest);
}

// ---------------------------------------------------------------- ports

/// Describe the repository a process runs in: its name, branch and latest commit.
fn explain_cwd(o: &Out, cwd: &Path) {
    let Some(top) = run("git", &["rev-parse", "--show-toplevel"], Some(cwd)) else {
        return;
    };
    let repo = Path::new(&top)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("?");
    let branch = run("git", &["branch", "--show-current"], Some(cwd))
        .unwrap_or_else(|| "detached HEAD".into());
    o.line(&format!(
        "that directory is inside the git repository `{repo}`, on `{branch}`"
    ));
    if let Some(c) =
        run("git", &["log", "-1", LOG_FMT], Some(cwd)).and_then(|r| parse_commits(&r).pop())
    {
        show_commit(o, "the latest commit is", &c);
    }
}

struct Proc {
    pid: u32,
    ppid: u32,
    user: String,
    started: String,
    command: String,
}

fn proc_info(pid: u32) -> Option<Proc> {
    let out = run(
        "ps",
        &["-o", "ppid=,user=,lstart=,command=", "-p", &pid.to_string()],
        None,
    )?;
    let mut it = out.split_whitespace();
    let ppid = it.next()?.parse().ok()?;
    let user = it.next()?.to_string();
    // `lstart` is always five fields: "Mon Sep 29 00:04:00 2026".
    let started = (0..5)
        .filter_map(|_| it.next())
        .collect::<Vec<_>>()
        .join(" ");
    let command = it.collect::<Vec<_>>().join(" ");
    Some(Proc {
        pid,
        ppid,
        user,
        started,
        command,
    })
}

fn shorten(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    format!("{cut}...")
}

fn explain_port(o: &Out, port: u16) -> bool {
    o.head(&format!("why :{port}"));
    let spec = format!("-iTCP:{port}");
    let Some(out) = run("lsof", &["-nP", &spec, "-sTCP:LISTEN", "-Fpcn"], None) else {
        o.line(&format!("nothing is listening on TCP port {port}"));
        return false;
    };

    let mut pid = None;
    let mut name = String::new();
    let mut addr = String::new();
    for l in out.lines() {
        match l.split_at(1) {
            ("p", v) if pid.is_none() => pid = v.parse::<u32>().ok(),
            ("c", v) if name.is_empty() => name = v.to_string(),
            ("n", v) if addr.is_empty() => addr = v.to_string(),
            _ => {}
        }
    }
    let Some(pid) = pid else {
        o.line(&format!("could not read the listener on port {port}"));
        return false;
    };

    o.line(&format!("`{name}` (pid {pid}) is listening on {addr}"));

    // Walk up the parent chain: each step answers "who started this?".
    let mut cur = pid;
    for depth in 0..MAX_CHAIN {
        let Some(p) = proc_info(cur) else { break };
        let who = if depth == 0 { "it" } else { "which" };
        o.line(&format!("{who} runs as {} since {}:", p.user, p.started));
        o.sub(&format!("[{}] {}", p.pid, shorten(&p.command, 100)));
        if p.ppid <= 1 {
            o.line(if p.ppid == 1 {
                "its parent is pid 1, so the init system (launchd/systemd) started it"
            } else {
                "it has no parent process"
            });
            break;
        }
        cur = p.ppid;
        let parent = proc_info(cur).map_or_else(|| "?".into(), |q| shorten(&q.command, 60));
        o.line(&format!("it was started by pid {cur} (`{parent}`)"));
    }

    if let Some(cwd) = run(
        "lsof",
        &["-a", "-p", &pid.to_string(), "-d", "cwd", "-Fn"],
        None,
    )
    .and_then(|s| {
        s.lines()
            .find_map(|l| l.strip_prefix('n').map(str::to_string))
    }) {
        o.line(&format!("it was started from `{cwd}`"));
        explain_cwd(o, Path::new(&cwd));
    }
    true
}

// ----------------------------------------------------------------- main

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let [arg] = args.as_slice() else {
        eprintln!("usage: why <path | command | :port>");
        return ExitCode::from(2);
    };
    if arg == "-h" || arg == "--help" {
        println!("usage: why <path | command | :port>");
        return ExitCode::SUCCESS;
    }

    let o = Out {
        color: std::io::stdout().is_terminal(),
    };
    match classify(arg) {
        Ok(Target::Port(p)) => {
            if explain_port(&o, p) {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        Ok(Target::Path(p)) => {
            explain_path(&o, &p, arg);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("why: {e}");
            ExitCode::from(2)
        }
    }
}
