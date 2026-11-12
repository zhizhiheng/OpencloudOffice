//! `oco corpus`: run the reader over a directory tree of documents.
//!
//! This is the robustness and coverage harness. Every file is opened and
//! fully resolved inside `catch_unwind`, so one bad document cannot hide the
//! results for the rest.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use oco_docx::{Docx, Error, OtherFormat};
use serde::Serialize;

pub struct Options {
    roots: Vec<PathBuf>,
    json: Option<PathBuf>,
    jobs: usize,
}

impl Options {
    pub fn parse(args: &[&str]) -> Option<Self> {
        let mut options = Options {
            roots: Vec::new(),
            json: None,
            jobs: std::thread::available_parallelism().map_or(1, usize::from),
        };
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            match *arg {
                "--json" => options.json = Some(PathBuf::from(args.next()?)),
                "--jobs" => options.jobs = args.next()?.parse().ok().filter(|n| *n > 0)?,
                flag if flag.starts_with("--") => return None,
                root => options.roots.push(PathBuf::from(root)),
            }
        }
        (!options.roots.is_empty()).then_some(options)
    }
}

#[derive(Serialize)]
struct FileReport {
    path: String,
    bytes: u64,
    micros: u64,
    #[serde(flatten)]
    outcome: Outcome,
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum Outcome {
    Ok {
        paragraphs: usize,
        runs: usize,
        chars: usize,
        tracked_runs: usize,
        /// Time spent computing effective formatting, out of `micros`.
        resolve_micros: u64,
        /// Non-fatal problems, e.g. a damaged styles part that was ignored.
        warnings: Vec<String>,
        unsupported: BTreeMap<String, usize>,
    },
    Error {
        kind: &'static str,
        message: String,
    },
    Panic {
        message: String,
    },
}

#[derive(Serialize)]
struct Unsupported {
    name: String,
    documents: usize,
    occurrences: usize,
}

#[derive(Serialize)]
struct Report {
    files: usize,
    opened: usize,
    /// Opened, but with a damaged optional part replaced by defaults.
    degraded: usize,
    failed: usize,
    panicked: usize,
    total_bytes: u64,
    /// Sum of per-file times, i.e. single-threaded cost.
    cpu_seconds: f64,
    /// Part of `cpu_seconds` spent computing effective formatting.
    resolve_seconds: f64,
    wall_seconds: f64,
    errors: BTreeMap<&'static str, usize>,
    unsupported: Vec<Unsupported>,
    results: Vec<FileReport>,
}

pub fn run(options: &Options) -> ExitCode {
    let mut files = Vec::new();
    for root in &options.roots {
        collect(root, &mut files);
    }
    files.sort();
    if files.is_empty() {
        eprintln!("oco: no Word files found");
        return ExitCode::FAILURE;
    }

    // Panics are recorded per file; the default hook would only add noise.
    let default_hook = panic::take_hook();
    panic::set_hook(Box::new(|_| {}));
    let started = Instant::now();
    let next = AtomicUsize::new(0);
    let results = Mutex::new(Vec::with_capacity(files.len()));
    std::thread::scope(|scope| {
        for _ in 0..options.jobs.min(files.len()) {
            scope.spawn(|| {
                while let Some(path) = files.get(next.fetch_add(1, Ordering::Relaxed)) {
                    let report = examine(path);
                    results.lock().unwrap().push(report);
                }
            });
        }
    });
    let wall_seconds = started.elapsed().as_secs_f64();
    panic::set_hook(default_hook);

    let mut results = results.into_inner().unwrap();
    results.sort_by(|a, b| a.path.cmp(&b.path));
    let report = summarise(results, wall_seconds);
    // Ignore a closed pipe: `oco corpus ... | head` is a normal way to use this.
    let _ = std::io::stdout().write_all(summary(&report).as_bytes());
    if let Some(path) = &options.json {
        let json = serde_json::to_string(&report).expect("report always serialises");
        if let Err(error) = fs::write(path, json) {
            eprintln!("oco: {}: {error}", path.display());
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}

fn collect(path: &Path, files: &mut Vec<PathBuf>) {
    if path.is_dir() {
        if path.file_name().is_some_and(|name| name == ".git") {
            return;
        }
        let Ok(entries) = fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            collect(&entry.path(), files);
        }
    } else if path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| ["docx", "docm", "dotx", "dotm"].contains(&e.to_ascii_lowercase().as_str()))
    {
        files.push(path.to_owned());
    }
}

fn examine(path: &Path) -> FileReport {
    let bytes = fs::metadata(path).map_or(0, |m| m.len());
    let started = Instant::now();
    let outcome = match panic::catch_unwind(AssertUnwindSafe(|| open_and_resolve(path))) {
        Ok(Ok(outcome)) => outcome,
        Ok(Err(error)) => Outcome::Error { kind: error_kind(&error), message: error.to_string() },
        Err(payload) => Outcome::Panic {
            message: payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                .unwrap_or_else(|| "panic".to_owned()),
        },
    };
    FileReport { path: path.display().to_string(), bytes, micros: started.elapsed().as_micros() as u64, outcome }
}

/// Opens the document and resolves every paragraph and run, which is the work
/// a layout engine would ask of the reader.
fn open_and_resolve(path: &Path) -> Result<Outcome, Error> {
    let docx = Docx::open(path)?;
    let resolving = Instant::now();
    let resolver = docx.resolver();
    let paragraphs = docx.document.paragraphs();
    let (mut runs, mut chars, mut tracked_runs) = (0, 0, 0);
    for paragraph in &paragraphs {
        std::hint::black_box(resolver.paragraph(&paragraph.props));
        for run in &paragraph.runs {
            std::hint::black_box(resolver.run(paragraph.props.style_id.as_deref(), &run.props));
            runs += 1;
            chars += run.text().chars().count();
            tracked_runs += usize::from(run.revision.is_some());
        }
    }
    Ok(Outcome::Ok {
        paragraphs: paragraphs.len(),
        runs,
        chars,
        tracked_runs,
        resolve_micros: resolving.elapsed().as_micros() as u64,
        warnings: docx.warnings.clone(),
        unsupported: docx.coverage.unsupported.clone(),
    })
}

fn error_kind(error: &Error) -> &'static str {
    match error {
        Error::Io(_) => "corrupt-data",
        Error::Zip(_) => "corrupt-zip",
        Error::Xml { .. } => "malformed-xml",
        Error::Encoding(_) => "encoding",
        Error::PartTooLarge { .. } => "part-too-large",
        Error::NoMainDocument => "no-main-document",
        Error::MissingPart(_) => "missing-part",
        Error::OtherFormat(OtherFormat::OleCompoundFile) => "ole-container",
        Error::OtherFormat(OtherFormat::OpenDocument) => "opendocument",
    }
}

fn summarise(results: Vec<FileReport>, wall_seconds: f64) -> Report {
    let mut errors = BTreeMap::new();
    let mut tally: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    let (mut opened, mut degraded, mut failed, mut panicked) = (0, 0, 0, 0);
    let mut resolve_micros = 0;
    for result in &results {
        match &result.outcome {
            Outcome::Ok { unsupported, warnings, resolve_micros: micros, .. } => {
                opened += 1;
                degraded += usize::from(!warnings.is_empty());
                resolve_micros += micros;
                for (name, count) in unsupported {
                    let entry = tally.entry(name).or_default();
                    entry.0 += 1;
                    entry.1 += count;
                }
            }
            Outcome::Error { kind, .. } => {
                failed += 1;
                *errors.entry(*kind).or_default() += 1;
            }
            Outcome::Panic { .. } => panicked += 1,
        }
    }
    let mut unsupported: Vec<Unsupported> = tally
        .into_iter()
        .map(|(name, (documents, occurrences))| Unsupported { name: name.to_owned(), documents, occurrences })
        .collect();
    // How many documents a gap affects matters more than how often it repeats.
    unsupported.sort_by(|a, b| b.documents.cmp(&a.documents).then(a.name.cmp(&b.name)));
    Report {
        files: results.len(),
        opened,
        degraded,
        failed,
        panicked,
        total_bytes: results.iter().map(|r| r.bytes).sum(),
        cpu_seconds: results.iter().map(|r| r.micros).sum::<u64>() as f64 / 1e6,
        resolve_seconds: resolve_micros as f64 / 1e6,
        wall_seconds,
        errors,
        unsupported,
        results,
    }
}

fn summary(report: &Report) -> String {
    let percent = |n: usize, of: usize| {
        if of == 0 {
            0.0
        } else {
            100.0 * n as f64 / of as f64
        }
    };
    let mut out = String::new();
    // Writing to a `String` cannot fail.
    macro_rules! line {
        ($($arg:tt)*) => { let _ = writeln!(out, $($arg)*); };
    }
    line!("files     {}", report.files);
    line!(
        "opened    {} ({:.1}%), {} of them degraded",
        report.opened,
        percent(report.opened, report.files),
        report.degraded
    );
    line!("failed    {}", report.failed);
    for (kind, count) in &report.errors {
        line!("  {count:>6}  {kind}");
    }
    line!("panicked  {}", report.panicked);
    for result in &report.results {
        if let Outcome::Panic { message } = &result.outcome {
            line!("  {}: {message}", result.path);
        }
    }

    let megabytes = report.total_bytes as f64 / 1e6;
    line!(
        "\ntime      {:.2}s cpu ({:.2}s of it resolving), {:.2}s wall, {:.0} MB/s per core",
        report.cpu_seconds,
        report.resolve_seconds,
        report.wall_seconds,
        megabytes / report.cpu_seconds.max(1e-9)
    );
    let mut micros: Vec<u64> = report.results.iter().map(|r| r.micros).collect();
    micros.sort_unstable();
    let at = |q: f64| micros[((micros.len() - 1) as f64 * q) as usize] as f64 / 1e3;
    line!("per file  median {:.2} ms, p95 {:.2} ms, p99 {:.2} ms, max {:.2} ms", at(0.5), at(0.95), at(0.99), at(1.0));
    let mut slowest: Vec<&FileReport> = report.results.iter().collect();
    slowest.sort_by_key(|r| std::cmp::Reverse(r.micros));
    for result in slowest.iter().take(5) {
        line!("  {:>9.1} ms  {:>8.1} MB  {}", result.micros as f64 / 1e3, result.bytes as f64 / 1e6, result.path);
    }

    line!("\nunmodelled markup, by documents affected ({} kinds):", report.unsupported.len());
    line!("  {:>6} {:>6} {:>10}  name", "docs", "%", "uses");
    for item in report.unsupported.iter().take(40) {
        line!(
            "  {:>6} {:>5.1}% {:>10}  {}",
            item.documents,
            percent(item.documents, report.opened),
            item.occurrences,
            item.name
        );
    }
    out
}
