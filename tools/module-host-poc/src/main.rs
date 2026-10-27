mod adapter;
mod apple_tv;
#[path = "../../mqtt-live-probe/src/enum_catalogue.rs"]
#[allow(dead_code)]
mod enum_catalogue;
mod layout;
#[path = "../../moteur-integre-banc/src/memory.rs"]
#[allow(dead_code)]
mod memory;
mod module;
mod music;
mod native;
mod runtime;
mod weather;
#[global_allocator]
#[cfg(not(feature = "plain-timing"))]
static ALLOCATOR: memory::Meter = memory::Meter;
#[global_allocator]
#[cfg(feature = "plain-timing")]
static ALLOCATOR: std::alloc::System = std::alloc::System;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args == ["--worker"] {
        return runtime::worker_from_stdin().await;
    }
    if args == ["--broker"] {
        return runtime::broker_service().await;
    }
    if args.len() == 2 && args[0] == "--plan" {
        let layout = runtime::read_layout((&args[1]).into())?;
        let groups:Vec<_>=layout.plan()?.iter().map(|p|serde_json::json!({"id":p.id,"modules":p.modules.iter().map(|m|&m.id).collect::<Vec<_>>()})).collect();
        println!(
            "{}",
            serde_json::json!({"mode":layout.mode,"groups":groups})
        );
        return Ok(());
    }
    if args.len() == 4 && args[0] == "--serve-group" {
        return runtime::serve_group((&args[1]).into(), &args[2], args[3].parse()?).await;
    }
    anyhow::ensure!(
        args.len() == 2 || args.len() == 4,
        "module-host-poc --config <layout.toml> [--fail-group <id>]"
    );
    anyhow::ensure!(args[0] == "--config", "configuration required");
    let fail = if args.len() == 4 {
        anyhow::ensure!(args[2] == "--fail-group", "unknown option");
        Some(args[3].as_str())
    } else {
        None
    };
    let layout = runtime::read_layout((&args[1]).into())?;
    let report = runtime::run(layout, fail).await?;
    println!("MODULE_REPORT {report}");
    Ok(())
}
