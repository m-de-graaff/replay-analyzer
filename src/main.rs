use std::fs::File;
use std::io::{self, BufWriter, IsTerminal, Read, Write};
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::Parser;
use replay_analyzer::matches::{FolderReport, find_match_folders};
use replay_analyzer::{Match, ReadMode, ReadOptions, Round, decompressed_bytes, file};

/// Parse Rainbow Six Siege replays (.rec files or match folders) into JSON.
#[derive(Parser)]
#[command(version)]
struct Cli {
    /// A .rec round file or a match folder. Reads stdin when omitted.
    input: Option<PathBuf>,
    /// Write output here instead of stdout.
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Pretty-print the JSON.
    #[arg(long)]
    pretty: bool,
    /// Print a short summary of the replay header.
    #[arg(long, conflicts_with = "dump")]
    info: bool,
    /// Write the raw decompressed replay (round files only).
    #[arg(short = 'p', long)]
    dump: bool,
    /// Only read the header and player list (round files only; faster).
    #[arg(long)]
    partial: bool,
    /// Add a census of every packet marker and property hash seen, known or
    /// not.
    #[arg(long)]
    census: bool,
    /// List the match folders under a folder (headers only, fast): rounds,
    /// missing and unfinished rounds, game sessions, duplicates, leftover
    /// temporary recordings, versions and file hashes.
    #[arg(long, conflicts_with_all = ["dump", "info", "partial", "census"])]
    list: bool,
    /// Every player across the match folders under a folder: stable key,
    /// name history, matches with and against you, and likely queue-mates.
    #[arg(long, conflicts_with_all = ["dump", "info", "partial", "census", "list"])]
    players: bool,
    /// Print the decoder profiles and tested builds: a stored round whose
    /// `(decoder, decoderRevision)` differs from its build's profile here
    /// would decode differently now.
    #[arg(long, conflicts_with_all = ["dump", "info", "partial", "census", "list", "players"])]
    decoders: bool,
    /// Log debug information to stderr.
    #[arg(short, long)]
    debug: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .with_writer(io::stderr)
        .with_env_filter(if cli.debug {
            "replay_analyzer=debug"
        } else {
            "replay_analyzer=warn"
        })
        .init();

    let is_dir = cli.input.as_ref().is_some_and(|p| p.is_dir());
    let mut out: Box<dyn Write> = match &cli.output {
        Some(path) => Box::new(BufWriter::new(
            File::create(path).with_context(|| format!("creating {}", path.display()))?,
        )),
        None => Box::new(BufWriter::new(io::stdout().lock())),
    };
    let options = ReadOptions {
        mode: if cli.partial {
            ReadMode::Partial
        } else {
            ReadMode::Full
        },
        census: cli.census,
    };

    if cli.decoders {
        write_json(&mut out, &replay_analyzer::decoder::table(), cli.pretty)?;
    } else if cli.players {
        let dir = cli.input.as_ref().filter(|_| is_dir);
        let dir = dir.context("--players needs a folder")?;
        write_json(&mut out, &players(dir)?, cli.pretty)?;
    } else if cli.list {
        let dir = cli.input.as_ref().filter(|_| is_dir);
        let dir = dir.context("--list needs a folder")?;
        let library = replay_analyzer::library::scan(dir, ReadMode::Header)
            .with_context(|| format!("no folder with .rec files under {}", dir.display()))?;
        write_json(&mut out, &library, cli.pretty)?;
    } else if is_dir {
        let dir = cli.input.as_ref().expect("is_dir implies input");
        if cli.dump || cli.partial {
            bail!("--dump and --partial need a single .rec file, not a folder");
        }
        let m = Match::open_with(dir, options)
            .with_context(|| format!("reading match {}", dir.display()))?;
        if cli.info {
            let first = m.rounds.first().context("match has no rounds")?;
            print_info(first);
            if let Some(summary) = m.summary() {
                print_summary(&summary);
            }
            if let Some(folder) = &m.folder {
                print_folder(folder);
            }
            return Ok(());
        }
        write_json(&mut out, &m, cli.pretty)?;
    } else {
        if let Some(p) = cli.input.as_ref().filter(|p| file::is_temporary(p)) {
            bail!(
                "{} is an in-progress recording (.tmprec), not a finished replay; \
                 only .rec files can be read",
                p.display()
            );
        }
        let raw = read_input(cli.input.as_ref())?;
        if cli.dump {
            out.write_all(&decompressed_bytes(&raw)?)?;
        } else {
            let mode = if cli.info {
                ReadMode::Header
            } else {
                options.mode
            };
            let mut round = Round::from_bytes(&raw, ReadOptions { mode, ..options })?;
            if let Some(path) = &cli.input {
                round.file = Some(replay_analyzer::FileInfo::new(path, &raw));
            }
            if cli.info {
                print_info(&round);
                return Ok(());
            }
            write_json(&mut out, &round, cli.pretty)?;
        }
    }
    out.flush()?;
    Ok(())
}

fn read_input(path: Option<&PathBuf>) -> Result<Vec<u8>> {
    match path {
        Some(p) => std::fs::read(p).with_context(|| format!("reading {}", p.display())),
        None => {
            let stdin = io::stdin();
            if stdin.is_terminal() {
                bail!("specify a replay file or folder (*.rec), or pipe one to stdin");
            }
            let mut buf = Vec::new();
            stdin.lock().read_to_end(&mut buf)?;
            Ok(buf)
        }
    }
}

fn write_json(out: &mut dyn Write, value: &impl serde::Serialize, pretty: bool) -> Result<()> {
    if pretty {
        serde_json::to_writer_pretty(&mut *out, value)?;
    } else {
        serde_json::to_writer(&mut *out, value)?;
    }
    writeln!(out)?;
    Ok(())
}

fn print_info(round: &Round) {
    let h = &round.header;
    let v = &round.version;
    let recorder = h.recording_player().map_or("N/A", |p| p.username.as_str());
    println!("Version:          {}/{}", h.game_version, h.code_version);
    if let Some(season) = &v.season {
        let branch = if v.branch.is_empty() {
            String::new()
        } else {
            format!(" ({})", v.branch)
        };
        println!("Season:           {season}{branch}, build {}", v.build);
    }
    let p = &round.parser;
    println!(
        "Decoder:          {} rev {} ({} {}){}",
        p.decoder,
        p.decoder_revision,
        p.parser,
        p.parser_version,
        if p.untested_build {
            " [untested build]"
        } else {
            ""
        }
    );
    let f = &round.format;
    println!(
        "Format:           {} v{}, {:?} layout, {} header properties",
        f.magic, f.format_version, f.layout, f.property_count
    );
    println!("Recording player: {recorder} [{}]", h.recording_profile_id);
    println!("Match ID:         {}", h.match_id);
    println!("Round:            {}", h.round_number + 1);
    println!(
        "Timestamp:        {} (recording PC local time)",
        h.timestamp.format("%Y-%m-%d %H:%M:%S")
    );
    if let Some(start) = h.start_time {
        println!("Started:          {start}");
    }
    if let Some(end) = h.end_time {
        println!("Ended:            {end}");
    }
    println!("Match type:       {}", h.match_type);
    println!("Game mode:        {}", h.game_mode);
    println!("Map:              {}", h.map);
    if let Some(t) = &round.timing {
        println!(
            "Frames:           {} over {:.1}s, {:.1}/s, {} gaps",
            t.frames,
            t.duration,
            t.sample_rate,
            t.gaps.len()
        );
    }
    if let Some(file) = &round.file {
        println!("File:             {} ({} bytes)", file.file_name, file.size);
        println!("SHA-256:          {}", file.sha256);
    }
}

fn print_summary(s: &replay_analyzer::MatchSummary) {
    use replay_analyzer::summary::Outcome;
    let team = |i: usize| s.teams[i].name.as_str();
    println!(
        "Queue:            {} ({}), map id {}",
        s.queue, s.match_type, s.map.id
    );
    let r = &s.rules;
    println!(
        "Rules:            first to {} of {}{}",
        r.rounds_to_win,
        r.rounds_per_match,
        r.overtime_rounds_to_win
            .map_or(String::new(), |ot| format!(", overtime first to {ot}"))
    );
    println!(
        "Score:            {} {} - {} {}{}",
        team(0),
        s.result.final_score[0],
        s.result.final_score[1],
        team(1),
        if s.result.overtime { " (overtime)" } else { "" }
    );
    let outcome = match (s.result.outcome, s.result.winner) {
        (Outcome::Decided, Some(w)) => format!("{} won", team(w)),
        (o, _) => format!("{o:?}"),
    };
    let yours = s
        .your_team
        .map_or(String::new(), |t| format!(", your team {}", team(t)));
    let early = if s.result.ended_early == Some(true) {
        ", ended early"
    } else {
        ""
    };
    println!("Result:           {outcome}{yours}{early}");
}

fn print_folder(f: &FolderReport) {
    if let Some(n) = &f.name {
        println!(
            "Folder:           created {} (local time), game process {}",
            n.local_time, n.process_id
        );
    }
    println!(
        "Rounds:           {:?} of {} files{}",
        f.rounds,
        f.round_files,
        match f.complete {
            Some(true) => ", match complete",
            Some(false) => ", match unfinished",
            None => "",
        }
    );
    if !f.missing_rounds.is_empty() {
        println!("Missing rounds:   {:?}", f.missing_rounds);
    }
    for s in &f.skipped {
        println!("Skipped:          {} ({})", s.file, s.reason);
    }
    for w in &f.warnings {
        println!("Warning:          {w}");
    }
}

/// The player directory for every match folder under `root`. Rounds are read
/// partially: enough for players, relations and parties.
fn players(root: &std::path::Path) -> Result<replay_analyzer::PlayerDirectory> {
    let mut summaries = Vec::new();
    for dir in find_match_folders(root)? {
        match Match::open_with(&dir, ReadMode::Partial) {
            Ok(m) => summaries.extend(m.summary()),
            Err(e) => tracing::warn!(path = %dir.display(), error = %e, "skipping match folder"),
        }
    }
    if summaries.is_empty() {
        bail!("no folder with .rec files under {}", root.display());
    }
    Ok(replay_analyzer::PlayerDirectory::new(&summaries))
}
