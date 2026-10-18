//! Test-only observer, copied to the synthetic container volume, never shipped in the image.
use std::{fs,io};
fn main()->io::Result<()> {
    let status=fs::read_to_string("/proc/1/status")?;
    let kib=|name:&str|status.lines().find(|l|l.starts_with(name)).and_then(|l|l.split_whitespace().nth(1)).and_then(|n|n.parse::<u64>().ok());
    let rss=kib("VmRSS:").ok_or_else(||io::Error::other("missing main process RSS"))?;
    let peak=kib("VmHWM:").ok_or_else(||io::Error::other("missing main process HWM"))?;
    let anon=kib("RssAnon:").unwrap_or(0);
    let cg=|path:&str|fs::read_to_string(path).ok().and_then(|s|s.trim().parse::<u64>().ok()).map_or_else(||"null".into(),|n|n.to_string());
    let events=fs::read_to_string("/sys/fs/cgroup/memory.events")?;
    let oom=events.lines().find(|l|l.starts_with("oom_kill ")).and_then(|l|l.split_whitespace().nth(1)).and_then(|n|n.parse::<u64>().ok()).unwrap_or(0);
    println!("{{\"rss_kib\":{rss},\"peak_rss_kib\":{peak},\"anon_kib\":{anon},\"cgroup_current_bytes\":{},\"cgroup_peak_bytes\":{},\"oom_kills\":{oom}}}",cg("/sys/fs/cgroup/memory.current"),cg("/sys/fs/cgroup/memory.peak"));
    Ok(())
}
