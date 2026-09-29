use std::{
    env, fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Instant,
};

fn compiler(
    cc: &str,
    args: &[String],
    source: &str,
    parent: &Path,
    mode: &[&str],
) -> Result<String, String> {
    let mut child = Command::new(cc)
        .args(["-x", "c", "-std=c11", "-iquote"])
        .arg(parent)
        .args(args)
        .args(mode)
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run {cc}: {e}"))?;
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    // Drain both streams while writing: large parsers can fill every pipe.
    let (write_result, out, err) = std::thread::scope(|scope| {
        let writer = scope.spawn(move || stdin.write_all(source.as_bytes()));
        let errors = scope.spawn(|| {
            let mut v = Vec::new();
            stderr.read_to_end(&mut v).map(|_| v)
        });
        let mut out = Vec::new();
        let result = stdout.read_to_end(&mut out).map(|_| out);
        (writer.join().unwrap(), result, errors.join().unwrap())
    });
    let status = child.wait().map_err(|e| e.to_string())?;
    let err = err.map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("{cc} failed:\n{}", String::from_utf8_lossy(&err)));
    }
    write_result.map_err(|e| e.to_string())?;
    String::from_utf8(out.map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

fn run() -> Result<(), String> {
    let mut input = None;
    let mut output: Option<PathBuf> = None;
    let mut cc = env::var("CC").unwrap_or_else(|_| "cc".into());
    let mut args = Vec::new();
    let mut rounds = 16;
    let mut timings = false;
    let mut jobs = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(8);
    let mut cli = env::args().skip(1);
    while let Some(arg) = cli.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!(
                    "Usage: tree-trimmer INPUT.c [-o OUTPUT.c] [--cc COMPILER] [--cpp-arg ARG] [--rounds N] [--jobs N] [--timings]\n\nCompress generated Tree-sitter C in place, or into OUTPUT.c when given.\nAlways verifies identical preprocessed tokens and C syntax before an atomic write.\nPass include paths and build defines with repeated --cpp-arg arguments.\nUse --rounds N to set search rounds (default: 16); 0 removes whitespace only. CC defaults to cc.\nUse --jobs N to set parallel workers (default: available CPUs, up to 8).\nUse --jobs 1 for serial execution; --timings reports each stage."
                );
                return Ok(());
            }
            "-o" => output = Some(cli.next().ok_or("missing output path")?.into()),
            "--cc" => cc = cli.next().ok_or("missing compiler")?,
            "--cpp-arg" => args.push(cli.next().ok_or("missing compiler argument")?),
            "--timings" => timings = true,
            "--jobs" => {
                jobs = cli
                    .next()
                    .ok_or("missing jobs")?
                    .parse::<usize>()
                    .map_err(|_| "invalid jobs")?;
                if jobs == 0 {
                    return Err("jobs must be greater than zero".into());
                }
            }
            "--rounds" => {
                rounds = cli
                    .next()
                    .ok_or("missing rounds")?
                    .parse::<usize>()
                    .map_err(|_| "invalid rounds")?
            }
            _ if arg.starts_with('-') => return Err(format!("unknown option: {arg}")),
            _ if input.is_none() => input = Some(PathBuf::from(arg)),
            _ => return Err("only one input file is supported".into()),
        }
    }
    let input = input
        .ok_or("usage: tree-trimmer INPUT.c [-o OUTPUT.c]")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let output = output.unwrap_or_else(|| input.clone());
    let output = output.canonicalize().unwrap_or(output);
    let source = fs::read_to_string(&input).map_err(|e| e.to_string())?;
    if !source.contains("tree_sitter/parser.h") {
        return Err(
            "expected a generated Tree-sitter parser including tree_sitter/parser.h".into(),
        );
    }
    for builtin in [
        "__LINE__",
        "__FILE__",
        "__BASE_FILE__",
        "__FILE_NAME__",
        "__COUNTER__",
        "__TIMESTAMP__",
        "__DATE__",
        "__TIME__",
    ] {
        if source.contains(builtin) {
            return Err(format!(
                "location/time-dependent builtin {builtin} is unsupported"
            ));
        }
    }
    let parent = input.parent().unwrap();
    let mut stage = Instant::now();
    eprintln!(
        "Compressing {} bytes with {jobs} worker(s); checking with {cc}...",
        source.len()
    );
    let (expanded, macros) = parallel_pair(
        jobs > 1,
        || compiler(&cc, &args, &source, parent, &["-E", "-P"]),
        || compiler(&cc, &args, &source, parent, &["-E", "-dM"]),
    )?;
    report_time(timings, "Preprocess original and read macros", &mut stage);
    let result = tree_trimmer::compress_with_jobs(&source, &[&expanded, &macros], rounds, jobs)?;
    report_time(timings, "Compress", &mut stage);
    parallel_pair(
        jobs > 1,
        || {
            let check = compiler(&cc, &args, &result.source, parent, &["-E", "-P"])?;
            tree_trimmer::equivalent(&expanded, &check)
        },
        || compiler(&cc, &args, &result.source, parent, &["-fsyntax-only"]),
    )?;
    report_time(timings, "Verify tokens and C syntax", &mut stage);
    drop(expanded);
    if output == input && fs::read_to_string(&input).map_err(|e| e.to_string())? != source {
        return Err("input changed during compression; output was not written".into());
    }
    atomic_write(&output, result.source.as_bytes())?;
    report_time(timings, "Write output", &mut stage);
    eprintln!(
        "{} → {} bytes ({:.1}% smaller); whitespace alone: {} bytes; {:.1}% smaller than whitespace alone; {} macros. Preprocessed tokens identical; C syntax checked.\n{}",
        source.len(),
        result.source.len(),
        100.0 * (1.0 - result.source.len() as f64 / source.len() as f64),
        result.whitespace_bytes,
        100.0 * (1.0 - result.source.len() as f64 / result.whitespace_bytes as f64),
        result.macros,
        output.display()
    );
    Ok(())
}

fn parallel_pair<A: Send, B: Send>(
    parallel: bool,
    left: impl FnOnce() -> Result<A, String>,
    right: impl FnOnce() -> Result<B, String> + Send,
) -> Result<(A, B), String> {
    if !parallel {
        return Ok((left()?, right()?));
    }
    std::thread::scope(|scope| {
        let worker = scope.spawn(right);
        let a = left();
        // Join even if the first operation failed. Both checks must finish
        // successfully before the caller can write anything.
        let b = worker.join().map_err(|_| "compiler worker panicked")?;
        Ok((a?, b?))
    })
}

fn report_time(enabled: bool, label: &str, stage: &mut Instant) {
    if enabled {
        eprintln!("  {label}: {:.3}s", stage.elapsed().as_secs_f64());
    }
    *stage = Instant::now();
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().unwrap_or(Path::new("."));
    let (temporary, mut file) = (0..100)
        .find_map(|attempt| {
            let p = parent.join(format!(
                ".tree-trimmer-{}-{attempt}.tmp",
                std::process::id()
            ));
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&p)
                .ok()
                .map(|f| (p, f))
        })
        .ok_or_else(|| format!("cannot create temporary output in {}", parent.display()))?;
    let result = (|| {
        file.write_all(bytes)?;
        if let Ok(metadata) = fs::metadata(path) {
            file.set_permissions(metadata.permissions())?;
        }
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|e| format!("cannot write {}: {e}", path.display()))
}

fn main() {
    if let Err(err) = run() {
        eprintln!("tree-trimmer: {err}");
        std::process::exit(1);
    }
}
