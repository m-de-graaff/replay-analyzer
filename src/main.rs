use std::fs::File;
use std::io::{self, BufWriter, IsTerminal, Read, Write};
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::Parser;
use replay_analyzer::{Header, Match, ReadMode, Round, decompressed_bytes};

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

    if is_dir {
        let dir = cli.input.as_ref().expect("is_dir implies input");
        if cli.dump || cli.partial {
            bail!("--dump and --partial need a single .rec file, not a folder");
        }
        let m = Match::open(dir).with_context(|| format!("reading match {}", dir.display()))?;
        if cli.info {
            let first = m.rounds.first().context("match has no rounds")?;
            return print_info(&first.header);
        }
        write_json(&mut out, &m, cli.pretty)?;
    } else {
        let raw = read_input(cli.input.as_ref())?;
        if cli.dump {
            out.write_all(&decompressed_bytes(&raw)?)?;
        } else if cli.info {
            let round = Round::from_bytes(&raw, ReadMode::Partial)?;
            return print_info(&round.header);
        } else {
            let mode = if cli.partial {
                ReadMode::Partial
            } else {
                ReadMode::Full
            };
            write_json(&mut out, &Round::from_bytes(&raw, mode)?, cli.pretty)?;
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

fn print_info(h: &Header) -> Result<()> {
    let recorder = h.recording_player().map_or("N/A", |p| p.username.as_str());
    println!("Version:          {}/{}", h.game_version, h.code_version);
    println!("Recording player: {recorder} [{}]", h.recording_profile_id);
    println!("Match ID:         {}", h.match_id);
    println!("Timestamp:        {}", h.timestamp);
    println!("Match type:       {}", h.match_type);
    println!("Game mode:        {}", h.game_mode);
    println!("Map:              {}", h.map);
    Ok(())
}
