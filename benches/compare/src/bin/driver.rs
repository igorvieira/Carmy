//! Load driver shared by every contender, so they all face the same client.
//!
//! ```text
//! driver mcp  --name NAME [options] -- <server command...>
//! driver http --name NAME --url URL [--env K=V]... [options] -- <server command...>
//! driver report <results.jsonl>
//! ```
//!
//! Each run spawns a fresh server, measures cold start, checks that the answer is
//! correct, warms up, then measures sequential latency, throughput under concurrency and
//! peak RSS. One JSON line per contender is printed, holding the median across runs.
use carmy_compare::{EXPECTED_MATCHES, QUERY};
use rmcp::{
    ServiceExt,
    model::{CallToolRequestParams, CallToolResponse},
};
use serde_json::{Value, json};
use std::{
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::process::{Child, Command};

type Error = Box<dyn std::error::Error + Send + Sync>;

struct Options {
    name: String,
    url: String,
    env: Vec<(String, String)>,
    runs: usize,
    calls: usize,
    warmup: usize,
    concurrency: usize,
    seconds: u64,
    command: Vec<String>,
}

#[derive(Default, Clone)]
struct Run {
    cold_start_ms: f64,
    seq_p50_us: f64,
    seq_p95_us: f64,
    seq_p99_us: f64,
    rps: f64,
    conc_p50_us: f64,
    conc_p95_us: f64,
    conc_p99_us: f64,
    peak_rss_mb: f64,
}

fn parse(args: &[String]) -> Result<Options, Error> {
    let split = args
        .iter()
        .position(|a| a == "--")
        .ok_or("missing `--` before the server command")?;
    let (flags, command) = (&args[..split], &args[split + 1..]);
    let mut o = Options {
        name: String::new(),
        url: String::new(),
        env: Vec::new(),
        runs: 3,
        calls: 5_000,
        warmup: 500,
        concurrency: 32,
        seconds: 10,
        command: command.to_vec(),
    };
    let mut it = flags.iter();
    while let Some(flag) = it.next() {
        let mut value = || it.next().cloned().ok_or(format!("{flag} needs a value"));
        match flag.as_str() {
            "--name" => o.name = value()?,
            "--url" => o.url = value()?,
            "--env" => {
                let kv = value()?;
                let (k, v) = kv.split_once('=').ok_or("--env takes K=V")?;
                o.env.push((k.into(), v.into()));
            }
            "--runs" => o.runs = value()?.parse()?,
            "--calls" => o.calls = value()?.parse()?,
            "--warmup" => o.warmup = value()?.parse()?,
            "--concurrency" => o.concurrency = value()?.parse()?,
            "--seconds" => o.seconds = value()?.parse()?,
            other => return Err(format!("unknown flag {other}").into()),
        }
    }
    if o.name.is_empty() || o.command.is_empty() {
        return Err("--name and a server command are required".into());
    }
    Ok(o)
}

fn spawn(o: &Options, piped: bool) -> Result<Child, Error> {
    let mut cmd = Command::new(&o.command[0]);
    cmd.args(&o.command[1..])
        .envs(o.env.iter().cloned())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    if piped {
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped());
    } else {
        cmd.stdin(Stdio::null()).stdout(Stdio::null());
    }
    Ok(cmd.spawn()?)
}

/// Samples the resident set size of `pid` (macOS and Linux `ps`), keeping the maximum.
fn rss_sampler(pid: u32) -> (Arc<AtomicBool>, Arc<AtomicU64>) {
    let stop = Arc::new(AtomicBool::new(false));
    let peak_kb = Arc::new(AtomicU64::new(0));
    let (s, p) = (stop.clone(), peak_kb.clone());
    std::thread::spawn(move || {
        while !s.load(Ordering::Relaxed) {
            if let Ok(out) = std::process::Command::new("ps")
                .args(["-o", "rss=", "-p", &pid.to_string()])
                .output()
                && let Ok(kb) = String::from_utf8_lossy(&out.stdout).trim().parse::<u64>()
            {
                p.fetch_max(kb, Ordering::Relaxed);
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    });
    (stop, peak_kb)
}

fn percentile(sorted: &[u64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let index = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[index] as f64
}

fn count_products(v: &Value) -> usize {
    v["products"].as_array().map_or(0, Vec::len)
}

// ---------------------------------------------------------------- MCP

fn mcp_params() -> CallToolRequestParams {
    CallToolRequestParams::new("search_products")
        .with_arguments(json!({ "query": QUERY }).as_object().unwrap().clone())
}

async fn mcp_call(peer: &rmcp::Peer<rmcp::RoleClient>) -> Result<usize, Error> {
    let CallToolResponse::Complete(result) = peer.call_tool_once(mcp_params()).await? else {
        return Err("unexpected MCP response".into());
    };
    if result.is_error == Some(true) {
        return Err(format!("tool error: {:?}", result.content).into());
    }
    let value = match result.structured_content {
        Some(v) => v,
        None => serde_json::from_str(&result.content[0].as_text().ok_or("no text content")?.text)?,
    };
    Ok(count_products(&value))
}

async fn mcp_run(o: &Options) -> Result<Run, Error> {
    let started = Instant::now();
    let mut child = spawn(o, true)?;
    let pid = child.id().ok_or("server exited")?;
    let (stop, peak) = rss_sampler(pid);
    let transport = (child.stdout.take().unwrap(), child.stdin.take().unwrap());
    let client = ().serve(transport).await?;
    let cold = started.elapsed();
    let peer = client.peer().clone();

    let found = mcp_call(&peer).await?;
    if found != EXPECTED_MATCHES {
        return Err(format!(
            "{}: expected {EXPECTED_MATCHES} products, got {found}",
            o.name
        )
        .into());
    }
    for _ in 0..o.warmup {
        mcp_call(&peer).await?;
    }
    let mut seq = Vec::with_capacity(o.calls);
    for _ in 0..o.calls {
        let t = Instant::now();
        mcp_call(&peer).await?;
        seq.push(t.elapsed().as_micros() as u64);
    }
    let (count, mut conc) = concurrent(o, {
        let peer = peer.clone();
        move || {
            let peer = peer.clone();
            async move { mcp_call(&peer).await.map(|_| ()) }
        }
    })
    .await?;
    stop.store(true, Ordering::Relaxed);
    drop(client);
    let _ = child.kill().await;
    Ok(summarize(
        cold,
        seq,
        count,
        &mut conc,
        o.seconds,
        peak.load(Ordering::Relaxed),
    ))
}

// ---------------------------------------------------------------- HTTP

async fn http_call(client: &reqwest::Client, url: &str) -> Result<usize, Error> {
    let body = json!({ "tool": "search_products", "arguments": { "query": QUERY } });
    let response = client.post(url).json(&body).send().await?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()).into());
    }
    let value: Value = response.json().await?;
    Ok(count_products(&value["data"]))
}

async fn http_run(o: &Options) -> Result<Run, Error> {
    let client = reqwest::Client::builder()
        .pool_max_idle_per_host(o.concurrency * 2)
        .build()?;
    let started = Instant::now();
    let mut child = spawn(o, false)?;
    let pid = child.id().ok_or("server exited")?;
    let (stop, peak) = rss_sampler(pid);
    let found = loop {
        match http_call(&client, &o.url).await {
            Ok(found) => break found,
            Err(_) if started.elapsed() < Duration::from_secs(30) => {
                tokio::time::sleep(Duration::from_millis(2)).await
            }
            Err(e) => return Err(format!("{} never became ready: {e}", o.name).into()),
        }
    };
    let cold = started.elapsed();
    if found != EXPECTED_MATCHES {
        return Err(format!(
            "{}: expected {EXPECTED_MATCHES} products, got {found}",
            o.name
        )
        .into());
    }
    for _ in 0..o.warmup {
        http_call(&client, &o.url).await?;
    }
    let mut seq = Vec::with_capacity(o.calls);
    for _ in 0..o.calls {
        let t = Instant::now();
        http_call(&client, &o.url).await?;
        seq.push(t.elapsed().as_micros() as u64);
    }
    let url = o.url.clone();
    let (count, mut conc) = concurrent(o, {
        let client = client.clone();
        move || {
            let (client, url) = (client.clone(), url.clone());
            async move { http_call(&client, &url).await.map(|_| ()) }
        }
    })
    .await?;
    stop.store(true, Ordering::Relaxed);
    let _ = child.kill().await;
    Ok(summarize(
        cold,
        seq,
        count,
        &mut conc,
        o.seconds,
        peak.load(Ordering::Relaxed),
    ))
}

// ---------------------------------------------------------------- shared

/// Runs `o.concurrency` workers calling `call` in a loop for `o.seconds`.
async fn concurrent<F, Fut>(o: &Options, call: F) -> Result<(u64, Vec<u64>), Error>
where
    F: Fn() -> Fut + Clone + Send + 'static,
    Fut: std::future::Future<Output = Result<(), Error>> + Send,
{
    let deadline = Instant::now() + Duration::from_secs(o.seconds);
    let workers = (0..o.concurrency).map(|_| {
        let call = call.clone();
        tokio::spawn(async move {
            let mut latencies = Vec::new();
            while Instant::now() < deadline {
                let t = Instant::now();
                call().await?;
                latencies.push(t.elapsed().as_micros() as u64);
            }
            Ok::<_, Error>(latencies)
        })
    });
    let mut all = Vec::new();
    for worker in futures_util::future::join_all(workers).await {
        all.extend(worker??);
    }
    Ok((all.len() as u64, all))
}

fn summarize(
    cold: Duration,
    mut seq: Vec<u64>,
    count: u64,
    conc: &mut [u64],
    seconds: u64,
    peak_kb: u64,
) -> Run {
    seq.sort_unstable();
    conc.sort_unstable();
    Run {
        cold_start_ms: cold.as_secs_f64() * 1000.0,
        seq_p50_us: percentile(&seq, 0.50),
        seq_p95_us: percentile(&seq, 0.95),
        seq_p99_us: percentile(&seq, 0.99),
        rps: count as f64 / seconds as f64,
        conc_p50_us: percentile(conc, 0.50),
        conc_p95_us: percentile(conc, 0.95),
        conc_p99_us: percentile(conc, 0.99),
        peak_rss_mb: peak_kb as f64 / 1024.0,
    }
}

fn median(runs: &[Run], field: impl Fn(&Run) -> f64) -> f64 {
    let mut values: Vec<f64> = runs.iter().map(field).collect();
    values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    values[values.len() / 2]
}

// ---------------------------------------------------------------- report

fn report(path: &str) -> Result<(), Error> {
    let text = std::fs::read_to_string(path)?;
    let rows: Vec<Value> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?;
    for kind in ["mcp", "http"] {
        println!(
            "\n### {}\n",
            if kind == "mcp" {
                "MCP over stdio"
            } else {
                "HTTP"
            }
        );
        println!(
            "| server | cold start | p50 | p95 | p99 | throughput | p95 under load | peak RSS |"
        );
        println!("|---|---|---|---|---|---|---|---|");
        for r in rows.iter().filter(|r| r["kind"] == kind) {
            let m = &r["median"];
            let f = |k: &str| m[k].as_f64().unwrap_or(0.0);
            println!(
                "| {} | {:.0} ms | {:.0} µs | {:.0} µs | {:.0} µs | {:.0} req/s | {:.0} µs | {:.1} MB |",
                r["name"].as_str().unwrap_or("?"),
                f("cold_start_ms"),
                f("seq_p50_us"),
                f("seq_p95_us"),
                f("seq_p99_us"),
                f("rps"),
                f("conc_p95_us"),
                f("peak_rss_mb"),
            );
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(mode) = args.first().cloned() else {
        return Err("usage: driver (mcp|http|report) …".into());
    };
    if mode == "report" {
        return report(args.get(1).ok_or("report needs a file")?);
    }
    let o = parse(&args[1..])?;
    let mut runs = Vec::new();
    // Run 0 primes the OS (e.g. the first-launch scan of a fresh binary) and is discarded.
    for run in 0..=o.runs {
        eprintln!(
            "[{}] run {run}/{}{}",
            o.name,
            o.runs,
            if run == 0 { " (priming)" } else { "" }
        );
        let result = match mode.as_str() {
            "mcp" => mcp_run(&o).await?,
            "http" => http_run(&o).await?,
            other => return Err(format!("unknown mode {other}").into()),
        };
        if run > 0 {
            runs.push(result);
        }
    }
    let pick = |f: fn(&Run) -> f64| median(&runs, f);
    let m = json!({
        "cold_start_ms": pick(|r| r.cold_start_ms),
        "seq_p50_us": pick(|r| r.seq_p50_us),
        "seq_p95_us": pick(|r| r.seq_p95_us),
        "seq_p99_us": pick(|r| r.seq_p99_us),
        "rps": pick(|r| r.rps),
        "conc_p50_us": pick(|r| r.conc_p50_us),
        "conc_p95_us": pick(|r| r.conc_p95_us),
        "conc_p99_us": pick(|r| r.conc_p99_us),
        "peak_rss_mb": pick(|r| r.peak_rss_mb),
    });
    println!(
        "{}",
        json!({ "kind": mode, "name": o.name, "runs": o.runs, "calls": o.calls,
                "concurrency": o.concurrency, "seconds": o.seconds, "median": m })
    );
    Ok(())
}
