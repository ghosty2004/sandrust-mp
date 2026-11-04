//! Headless test client: cookie exchange → RakNet connection → auth key →
//! join (RPC 25) → InitGame (RPC 139) → class selection (128) → spawn
//! (129 → 52), then one of the scripted modes (default: walk a small circle
//! around the spawn point). Prints chat, game texts, dialogs and player
//! joins/quits.

use std::time::{Duration, Instant};

use samp_bot::bot::{Bot, Config, Mode, PlayerRef};
use samp_bot::client::Client;
use samp_bot::path::Shape;
use samp_bot::{Result, geom};
use samp_proto::auth::AuthTable;

const USAGE: &str = "\
usage: samp-bot [HOST:PORT] [options]
  --name NAME          nickname (default RustBot)
  --seconds N          session length (default 15); a seated bot leaves its vehicle first
  --auth-table PATH    default data/auth_table.txt
  --class N            class to request (default 0)
  --mode MODE          walk (default) | idle | drive | passenger | shoot
  --sync-ms MS         fixed foot/in-car sync interval instead of rate + N (measurements)
 drive / passenger:
  --vehicle ID         vehicle to enter (default: nearest streamed, undriven)
  --seat N             passenger seat for 211 (default 1)
  --path circle|line   circle (default) or 50 m straight + 90° left turn
  --radius M           circle / turn radius (default 10 / 20)
  --speed M/S          cruise speed (default 5)
  --keys N             extra key bits held while driving (2 horn, 128 handbrake)
  --siren              siren on
  --drive-by           passenger drive-by (211 bit 6)
  --exit-after S       leave the vehicle (RPC 154) S seconds after sitting down
 shoot:
  --target ID|NAME     player to shoot at
  --weapon W           weapon to use (must be given by the server; default: armed)
  --hit-type T         206 hit type 0 none, 1 player (default), 2 vehicle, 3 object
  --hit-id N           206 hit id override
  --offset X,Y,Z       206 offset override
  --fire-ms MS         time between shots (default per weapon)";

struct Args {
    target: String,
    seconds: u64,
    auth_table: String,
    cfg: Config,
}

fn parse_args() -> std::result::Result<Args, String> {
    let mut a = Args {
        target: "127.0.0.1:7777".into(),
        seconds: 15,
        auth_table: "data/auth_table.txt".into(),
        cfg: Config::default(),
    };
    let mut path = "circle".to_owned();
    let mut radius: Option<f32> = None;
    let mut it = std::env::args().skip(1);
    fn num<T: std::str::FromStr>(flag: &str, v: Option<String>) -> std::result::Result<T, String> {
        v.and_then(|s| s.parse().ok()).ok_or_else(|| format!("{flag} needs a number"))
    }
    while let Some(arg) = it.next() {
        let c = &mut a.cfg;
        match arg.as_str() {
            "--name" => c.name = it.next().ok_or("--name needs a value")?,
            "--seconds" => a.seconds = num("--seconds", it.next())?,
            "--auth-table" => a.auth_table = it.next().ok_or("--auth-table needs a path")?,
            "--class" => c.class = num("--class", it.next())?,
            "--mode" => c.mode = it.next().ok_or("--mode needs a value")?.parse()?,
            "--sync-ms" => c.sync_ms = Some(num("--sync-ms", it.next())?),
            "--vehicle" => c.vehicle = Some(num("--vehicle", it.next())?),
            "--seat" => c.seat = num("--seat", it.next())?,
            "--path" => path = it.next().ok_or("--path needs a value")?,
            "--radius" => radius = Some(num("--radius", it.next())?),
            "--speed" => c.speed = num("--speed", it.next())?,
            "--keys" => c.extra_keys = num("--keys", it.next())?,
            "--siren" => c.siren = true,
            "--drive-by" => c.drive_by = true,
            "--exit-after" => c.exit_after = Some(Duration::from_secs_f32(num("--exit-after", it.next())?)),
            "--target" => {
                let t = it.next().ok_or("--target needs an id or a name")?;
                c.target = Some(t.parse().map_or(PlayerRef::Name(t), PlayerRef::Id));
            }
            "--weapon" => c.weapon = Some(num("--weapon", it.next())?),
            "--hit-type" => c.hit_type = num("--hit-type", it.next())?,
            "--hit-id" => c.hit_id = Some(num("--hit-id", it.next())?),
            "--offset" => {
                let v = it.next().ok_or("--offset needs X,Y,Z")?;
                let p: Vec<f32> = v.split(',').filter_map(|s| s.trim().parse().ok()).collect();
                let [x, y, z] = p[..] else { return Err("--offset needs X,Y,Z".into()) };
                c.offset = Some(geom::v3(x, y, z));
            }
            "--fire-ms" => c.fire_ms = Some(num("--fire-ms", it.next())?),
            "-h" | "--help" => return Err(USAGE.into()),
            other if other.starts_with("--") => return Err(format!("unknown option {other}\n{USAGE}")),
            other => a.target = other.into(),
        }
    }
    a.cfg.shape = match path.as_str() {
        "circle" => Shape::Circle { radius: radius.unwrap_or(10.0) },
        "line" => Shape::LineTurn { straight: 50.0, turn_radius: radius.unwrap_or(20.0) },
        p => return Err(format!("unknown path {p:?} (circle|line)")),
    };
    if a.cfg.mode == Mode::Shoot && a.cfg.target.is_none() {
        return Err("--mode shoot needs --target".into());
    }
    Ok(a)
}

fn main() -> Result<()> {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let table = AuthTable::parse(&std::fs::read_to_string(&args.auth_table).map_err(|e| {
        format!("cannot read auth table {}: {e} (generate it with tools/extract_auth_table.py)", args.auth_table)
    })?)
    .map_err(|e| format!("auth table parse error at line {}", e.line))?;

    let mut client = Client::connect(&args.target, &args.cfg.name, table)?;
    let start = Instant::now();
    let mut bot = Bot::new(args.cfg, start);
    let deadline = start + Duration::from_secs(args.seconds);
    // Grace period to leave a vehicle after the deadline.
    let grace = Duration::from_secs(6);
    loop {
        let now = Instant::now();
        if now >= deadline {
            bot.request_shutdown();
            if bot.ready_to_quit() || now >= deadline + grace {
                break;
            }
        }
        bot.tick(now, &mut client);
        for ev in client.pump(Duration::from_millis(5))? {
            if !bot.handle_event(ev, Instant::now(), &mut client)? {
                return Ok(());
            }
        }
    }
    client.disconnect()?;
    println!("done ({}s, phase {:?}), disconnected", args.seconds, bot.phase);
    Ok(())
}
