//! Layout and cache-sharing experiments. No production storage is changed.
#[path = "../layouts.rs"] mod layouts;
#[path = "../model.rs"] mod model;
use layouts::*;
use model::{Bank, FAMILY, HOUSE, PHYSICAL, VALUE, INTEGER, HAS_OBSERVATION, HAS_PREVIOUS, PREVIOUS_KIND_MASK, PREVIOUS_KIND_SHIFT};
use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::mem::{align_of, size_of};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::time::Instant;

struct Meter;
static LIVE: AtomicUsize = AtomicUsize::new(0);
#[global_allocator] static ALLOC: Meter = Meter;
unsafe impl GlobalAlloc for Meter {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(l) };
        if !p.is_null() { LIVE.fetch_add(l.size(), Ordering::Relaxed); }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        LIVE.fetch_sub(l.size(), Ordering::Relaxed);
        unsafe { System.dealloc(p,l); }
    }
}

#[repr(C, align(128))] struct Block96 { rows: [Row96; 4] }
#[repr(C, align(64))] struct Padded64 { row: Row96, spare: [u8;32] }
#[repr(C, align(128))] struct Padded128 { row: Row96, spare: [u8;32] }
#[repr(C, align(128))] struct Inline128 { row: Row96, meta: Meta32 }

fn seed(i: usize) -> Row96 {
    Row96 { base: Row64 { id:i as u64+1, parent:1, payload:(i%101) as u64,
        previous_payload:((i+100)%101) as u64, received_ms:1000, changed_ms:990,
        schema:2, aux:3, revision:2, flags:HAS_OBSERVATION|HAS_PREVIOUS|0x1000,
        row_kind:VALUE, value_kind:INTEGER }, numeric:Numeric32 {min:0,max:100,step:1,anchor:0} }
}
fn meta(i: usize) -> Meta32 { Meta32 {label:i as u32, native_key:2, location:3, policy:4, read_plan:5, write_plan:6, executable:7, extra:8} }
fn transition(mut row: Row64, value: u64, at: u64) -> Row64 {
    if row.payload != value {
        row.previous_payload=row.payload;
        row.flags=(row.flags & !PREVIOUS_KIND_MASK)|HAS_PREVIOUS|(u16::from(row.value_kind)<<PREVIOUS_KIND_SHIFT);
        row.payload=value; row.changed_ms=at;
    }
    row.received_ms=at+3; row.revision=row.revision.checked_add(1).unwrap(); row
}
fn hot(r: Row64)->u64 { r.payload.wrapping_mul(31)^r.previous_payload.wrapping_mul(17)^r.received_ms^r.changed_ms^u64::from(r.revision)^u64::from(r.flags) }
fn cold(r: Row96)->u64 { hot(r.base)^r.base.id^r.base.parent^u64::from(r.base.schema)^u64::from(r.base.aux)^r.numeric.min^r.numeric.max^r.numeric.step^r.numeric.anchor }
fn meta_digest(m: Meta32)->u64 { [m.label,m.native_key,m.location,m.policy,m.read_plan,m.write_plan,m.executable,m.extra].iter().map(|x|u64::from(*x)).sum() }
fn footprint(name:&str,len:usize,cap:usize,bytes:usize,ptr:usize)->String {
    format!("{{\"name\":\"{name}\",\"len\":{len},\"capacity\":{cap},\"element_bytes\":{bytes},\"used_bytes\":{},\"reserved_bytes\":{},\"base_mod64\":{},\"base_mod128\":{}}}",len*bytes,cap*bytes,ptr%64,ptr%128)
}
trait Store {
    fn new(n:usize)->Self;
    fn row(&self,i:usize)->Row96;
    fn set(&mut self,i:usize,r:Row64);
    fn metadata(&self,i:usize)->Meta32;
    fn arrays(&self)->Vec<String>;
    fn row_address(&self,i:usize)->usize;
}
struct Compact { rows:Vec<Row96>, meta:Vec<Meta32> }
impl Store for Compact {
    fn new(n:usize)->Self { Self{rows:(0..n).map(seed).collect(),meta:(0..n).map(meta).collect()} }
    #[inline] fn row(&self,i:usize)->Row96 {self.rows[i]}
    #[inline] fn set(&mut self,i:usize,r:Row64){self.rows[i].base=r;}
    #[inline] fn metadata(&self,i:usize)->Meta32{self.meta[i]}
    fn arrays(&self)->Vec<String>{vec![footprint("Row96",self.rows.len(),self.rows.capacity(),96,self.rows.as_ptr() as usize),footprint("Meta32",self.meta.len(),self.meta.capacity(),32,self.meta.as_ptr() as usize)]}
    fn row_address(&self,i:usize)->usize{&self.rows[i] as *const Row96 as usize}
}
struct Base128 { blocks:Vec<Block96>, meta:Vec<Meta32> }
impl Store for Base128 {
    fn new(n:usize)->Self{Self{blocks:(0..n.div_ceil(4)).map(|b|Block96{rows:std::array::from_fn(|i|seed(b*4+i))}).collect(),meta:(0..n).map(meta).collect()}}
    #[inline] fn row(&self,i:usize)->Row96{self.blocks[i/4].rows[i%4]}
    #[inline] fn set(&mut self,i:usize,r:Row64){self.blocks[i/4].rows[i%4].base=r;}
    #[inline] fn metadata(&self,i:usize)->Meta32{self.meta[i]}
    fn arrays(&self)->Vec<String>{vec![footprint("Block4x96",self.blocks.len(),self.blocks.capacity(),384,self.blocks.as_ptr() as usize),footprint("Meta32",self.meta.len(),self.meta.capacity(),32,self.meta.as_ptr() as usize)]}
    fn row_address(&self,i:usize)->usize{&self.blocks[i/4].rows[i%4] as *const Row96 as usize}
}
macro_rules! padded {
    ($name:ident,$row:ident) => {
        struct $name {rows:Vec<$row>,meta:Vec<Meta32>}
        impl Store for $name {
            fn new(n:usize)->Self{Self{rows:(0..n).map(|i|$row{row:seed(i),spare:[0;32]}).collect(),meta:(0..n).map(meta).collect()}}
            #[inline] fn row(&self,i:usize)->Row96{self.rows[i].row}
            #[inline] fn set(&mut self,i:usize,r:Row64){self.rows[i].row.base=r;}
            #[inline] fn metadata(&self,i:usize)->Meta32{self.meta[i]}
            fn arrays(&self)->Vec<String>{vec![footprint("Row128",self.rows.len(),self.rows.capacity(),128,self.rows.as_ptr() as usize),footprint("Meta32",self.meta.len(),self.meta.capacity(),32,self.meta.as_ptr() as usize)]}
            fn row_address(&self,i:usize)->usize{&self.rows[i] as *const $row as usize}
        }
    }
}
padded!(Pad64,Padded64);
padded!(Pad128,Padded128);
struct Integrated {rows:Vec<Inline128>}
impl Store for Integrated {
    fn new(n:usize)->Self{Self{rows:(0..n).map(|i|Inline128{row:seed(i),meta:meta(i)}).collect()}}
    #[inline] fn row(&self,i:usize)->Row96{self.rows[i].row}
    #[inline] fn set(&mut self,i:usize,r:Row64){self.rows[i].row.base=r;}
    #[inline] fn metadata(&self,i:usize)->Meta32{self.rows[i].meta}
    fn arrays(&self)->Vec<String>{vec![footprint("Row96+Meta32",self.rows.len(),self.rows.capacity(),128,self.rows.as_ptr() as usize)]}
    fn row_address(&self,i:usize)->usize{&self.rows[i] as *const Inline128 as usize}
}
struct Split {hot:Vec<Hot32>,cold:Vec<Cold32>,numeric:Vec<Numeric32>,meta:Vec<Meta32>}
impl Store for Split {
    fn new(n:usize)->Self{let mut s=Self{hot:Vec::with_capacity(n),cold:Vec::with_capacity(n),numeric:Vec::with_capacity(n),meta:Vec::with_capacity(n)};for i in 0..n{let r=seed(i);s.hot.push(Hot32{payload:r.base.payload,previous_payload:r.base.previous_payload,received_ms:r.base.received_ms,changed_ms:r.base.changed_ms});s.cold.push(Cold32{id:r.base.id,parent:r.base.parent,schema:r.base.schema,aux:r.base.aux,revision:r.base.revision,flags:r.base.flags,row_kind:r.base.row_kind,value_kind:r.base.value_kind});s.numeric.push(r.numeric);s.meta.push(meta(i));}s}
    #[inline] fn row(&self,i:usize)->Row96{let h=self.hot[i];let c=self.cold[i];Row96{base:Row64{id:c.id,parent:c.parent,payload:h.payload,previous_payload:h.previous_payload,received_ms:h.received_ms,changed_ms:h.changed_ms,schema:c.schema,aux:c.aux,revision:c.revision,flags:c.flags,row_kind:c.row_kind,value_kind:c.value_kind},numeric:self.numeric[i]}}
    #[inline] fn set(&mut self,i:usize,r:Row64){self.hot[i]=Hot32{payload:r.payload,previous_payload:r.previous_payload,received_ms:r.received_ms,changed_ms:r.changed_ms};self.cold[i]=Cold32{id:r.id,parent:r.parent,schema:r.schema,aux:r.aux,revision:r.revision,flags:r.flags,row_kind:r.row_kind,value_kind:r.value_kind};}
    #[inline] fn metadata(&self,i:usize)->Meta32{self.meta[i]}
    fn arrays(&self)->Vec<String>{vec![footprint("Hot32",self.hot.len(),self.hot.capacity(),32,self.hot.as_ptr() as usize),footprint("Cold32",self.cold.len(),self.cold.capacity(),32,self.cold.as_ptr() as usize),footprint("Numeric32",self.numeric.len(),self.numeric.capacity(),32,self.numeric.as_ptr() as usize),footprint("Meta32",self.meta.len(),self.meta.capacity(),32,self.meta.as_ptr() as usize)]}
    fn row_address(&self,i:usize)->usize{&self.hot[i] as *const Hot32 as usize}
}
fn semantic<S:Store>() {
    let mut store=S::new(8);
    for i in 0..8 {assert_eq!(store.row(i),seed(i));assert_eq!(store.metadata(i),meta(i));}
    let mut bank=Bank::new();bank.insert(1,FAMILY,0,0).unwrap();bank.insert(2,HOUSE,1,0).unwrap();bank.insert(3,PHYSICAL,2,0).unwrap();bank.insert(4,VALUE,3,0).unwrap();
    // Start from the same observed state; compare all state words against real Bank.
    let position=bank.slots[bank.handle(4).unwrap().slot as usize].physical_index as usize;
    bank.rows[position].base=store.row(3).base;bank.rows[position].base.parent=3;
    for (step,value) in [21,22,22,23].into_iter().enumerate(){let at=2000+step as u64;bank.observe(4,INTEGER,value,at,at+3).unwrap();let next=transition(store.row(3).base,value,at);store.set(3,next);let mut expected=bank.read_at(4,1).unwrap().base;expected.parent=1;assert_eq!(store.row(3).base,expected);}
}
fn timed<F:FnMut(usize)->u64>(variant:&str,op:&str,iterations:usize,mut action:F){
    for i in 0..iterations.min(1024){black_box(action(black_box(i)));}
    for round in 0..3{let before=LIVE.load(Ordering::Relaxed);let start=Instant::now();let mut sum=0u64;for i in 0..iterations{sum=sum.wrapping_add(black_box(action(black_box(i))));}let ns=start.elapsed().as_nanos();black_box(sum);assert_eq!(before,LIVE.load(Ordering::Relaxed));println!("{{\"kind\":\"sample\",\"variant\":\"{variant}\",\"operation\":\"{op}\",\"round\":{round},\"iterations\":{iterations},\"elapsed_ns\":{ns},\"ns_per_op\":{},\"checksum\":{sum}}}",ns as f64/iterations as f64);}
}
fn memory_status()->String{let raw=std::fs::read_to_string("/proc/self/status").unwrap();let entries:Vec<_>=raw.lines().filter_map(|line|{let (key,value)=line.split_once(':')?;if ["VmRSS","RssAnon","VmHWM"].contains(&key){let bytes=value.split_whitespace().next()?.parse::<usize>().ok()?*1024;Some(format!("\"{key}\":{bytes}"))}else{None}}).collect();format!("{{{}}}",entries.join(","))}
fn serial<S:Store>(variant:&str,n:usize){
    semantic::<S>();let before=LIVE.load(Ordering::Relaxed);let mut store=S::new(n);let reserved=LIVE.load(Ordering::Relaxed)-before;
    let mut index=std::collections::HashMap::with_capacity(n);for i in 0..n{index.insert(i as u64+1,i as u32);}let idx_bytes=LIVE.load(Ordering::Relaxed)-before-reserved;
    let slots=vec![GlobalSlot16::default();n];black_box(&slots);
    println!("{{\"kind\":\"memory\",\"variant\":\"{variant}\",\"n\":{n},\"requested_rows_meta_bytes\":{reserved},\"index_requested_bytes\":{idx_bytes},\"slots_bytes\":{},\"arrays\":[{}],\"process_bytes\":{}}}",slots.capacity()*16,store.arrays().join(","),memory_status());
    let queries:Vec<_>=(0..8192).map(|i|((i as u64).wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407)%n as u64) as usize).collect();
    timed(variant,"scan_state",n*3,|i|hot(store.row(i%n).base));
    timed(variant,"scan_full",n*3,|i|cold(store.row(i%n)));
    timed(variant,"random_state",32768,|i|hot(store.row(queries[i%queries.len()]).base));
    timed(variant,"id_state",32768,|i|{let id=queries[i%queries.len()] as u64+1;hot(store.row(*index.get(black_box(&id)).unwrap() as usize).base)});
    timed(variant,"state_and_meta",32768,|i|{let j=queries[i%queries.len()];hot(store.row(j).base)^meta_digest(store.metadata(j))});
    timed(variant,"update_previous",32768,|i|{let j=queries[i%queries.len()];let r=transition(store.row(j).base,black_box(i as u64+300),black_box(i as u64+2000));store.set(j,r);hot(store.row(j).base)});
}

unsafe extern "C" {fn sched_setaffinity(pid:i32,size:usize,mask:*const u8)->i32;fn sched_getcpu()->i32;}
fn pin(cpu:usize){let mut mask=[0u8;128];mask[cpu/8]|=1<<(cpu%8);assert_eq!(unsafe{sched_setaffinity(0,mask.len(),mask.as_ptr())},0,"CPU affinity unavailable");}
#[repr(C)] struct Atom96{words:[AtomicU64;12]}
#[repr(C,align(64))] struct Atom128a64{words:[AtomicU64;12],pad:[u8;32]}
#[repr(C,align(128))] struct Atom128a128{words:[AtomicU64;12],pad:[u8;32]}
#[repr(C)] struct AtomHot32{words:[AtomicU64;4]}
#[repr(C,align(128))] struct AtomHot128{words:[AtomicU64;4],pad:[u8;96]}
trait Atom:Sync{fn new()->Self;fn word(&self,i:usize)->&AtomicU64;fn hot_offset()->usize;}
macro_rules! atom {($t:ident,$len:expr,$offset:expr,$($rest:tt)*)=>{impl Atom for $t{fn new()->Self{Self{words:std::array::from_fn(|_|AtomicU64::new(10)),$($rest)*}}#[inline]fn word(&self,i:usize)->&AtomicU64{&self.words[i]}fn hot_offset()->usize{$offset}}};}
atom!(Atom96,12,2,);
atom!(Atom128a64,12,2,pad:[0;32]);
atom!(Atom128a128,12,2,pad:[0;32]);
atom!(AtomHot32,4,0,);
atom!(AtomHot128,4,0,pad:[0;96]);
fn concurrent<A:Atom>(variant:&str,threads:usize,pattern:&str,all_words:bool){
    let per_thread=256usize;let n=per_thread*threads;let iterations=262144usize;
    for round in 0..3{
        let data:Vec<A>=(0..n).map(|_|A::new()).collect();let base=data.as_ptr() as usize;
        let barrier=Arc::new(Barrier::new(threads+1));
        let (elapsed,witness)=std::thread::scope(|scope|{
            let workers:Vec<_>=(0..threads).map(|thread|{let b=barrier.clone();let data=&data;scope.spawn(move||{
                pin(thread);let cpu_start=unsafe{sched_getcpu()};b.wait();b.wait();
                for i in 0..iterations{let local=i%per_thread;let j=if pattern=="interleaved"{local*threads+thread}else{thread*per_thread+local};let value=(i/per_thread) as u64;let row=&data[j];
                    if all_words {for word in 0..12 {row.word(word).store(black_box(value+word as u64),Ordering::Relaxed);}}
                    else {let at=A::hot_offset();let old=row.word(at).load(Ordering::Relaxed);row.word(at+1).store(old,Ordering::Relaxed);row.word(at).store(black_box(value),Ordering::Relaxed);row.word(at+2).store(i as u64+2000,Ordering::Relaxed);row.word(at+3).store(i as u64+1990,Ordering::Relaxed);}
                }
                b.wait();(thread,cpu_start,unsafe{sched_getcpu()})
            })}).collect();
            barrier.wait();let start=Instant::now();barrier.wait();barrier.wait();let elapsed=start.elapsed().as_nanos();let witness:Vec<_>=workers.into_iter().map(|w|w.join().unwrap()).collect();(elapsed,witness)
        });
        for (cpu,start,end) in &witness{assert_eq!(*start,*cpu as i32);assert_eq!(*end,*cpu as i32);}
        let sum=data.iter().map(|a|a.word(A::hot_offset()).load(Ordering::Relaxed)).sum::<u64>();
        let witness:Vec<_>=witness.iter().map(|(cpu,start,end)|format!("{{\"requested\":{cpu},\"start\":{start},\"end\":{end}}}")).collect();
        let op=if all_words{"all_words"}else{"state_words"};let total=iterations*threads;
        println!("{{\"kind\":\"parallel\",\"variant\":\"{variant}\",\"operation\":\"{op}\",\"pattern\":\"{pattern}\",\"threads\":{threads},\"round\":{round},\"iterations\":{total},\"elapsed_ns\":{elapsed},\"ns_per_op\":{},\"checksum\":{sum},\"row_bytes\":{},\"base_mod64\":{},\"base_mod128\":{},\"hot_offset\":{},\"worker_cpus\":[{}]}}",elapsed as f64/total as f64,size_of::<A>(),base%64,base%128,A::hot_offset()*8,witness.join(","));
    }
}
fn self_test(){
    assert_eq!((size_of::<Row96>(),align_of::<Row96>()),(96,8));assert_eq!((size_of::<Block96>(),align_of::<Block96>()),(384,128));
    assert_eq!((size_of::<Padded64>(),align_of::<Padded64>()),(128,64));assert_eq!((size_of::<Padded128>(),align_of::<Padded128>()),(128,128));assert_eq!((size_of::<Inline128>(),align_of::<Inline128>()),(128,128));
    semantic::<Compact>();semantic::<Base128>();semantic::<Pad64>();semantic::<Pad128>();semantic::<Integrated>();semantic::<Split>();
    let b=Base128::new(9);assert_eq!(b.row_address(0)%128,0);assert_eq!(b.row_address(1)-b.row_address(0),96);assert_eq!(b.row_address(4)-b.row_address(3),96);
    let p=Integrated::new(9);assert_eq!(p.row_address(0)%128,0);assert_eq!(p.row_address(1)-p.row_address(0),128);
    assert_eq!([size_of::<Atom96>(),size_of::<Atom128a64>(),size_of::<Atom128a128>(),size_of::<AtomHot32>(),size_of::<AtomHot128>()],[96,128,128,32,128]);
    println!("{{\"kind\":\"tests\",\"passed\":true,\"layout_variants\":6,\"atomic_variants\":5,\"parity_against_bank\":true,\"sequence\":[21,22,22,23]}}");
}
fn main(){
    let args:Vec<_>=std::env::args().collect();match args.get(1).map(String::as_str){
        Some("--self-test")=>self_test(),
        Some("--serial")=>{pin(0);let n=args[3].parse().unwrap();match args[2].as_str(){"compact96"=>serial::<Compact>(&args[2],n),"base128_stride96"=>serial::<Base128>(&args[2],n),"pad128_align64"=>serial::<Pad64>(&args[2],n),"pad128_align128"=>serial::<Pad128>(&args[2],n),"inline_meta128"=>serial::<Integrated>(&args[2],n),"split32"=>serial::<Split>(&args[2],n),_=>panic!("unknown layout")}},
        Some("--parallel")=>{let threads=args[3].parse().unwrap();let all=args[5]=="all_words";match args[2].as_str(){"compact96"=>concurrent::<Atom96>(&args[2],threads,&args[4],all),"pad128_align64"=>concurrent::<Atom128a64>(&args[2],threads,&args[4],all),"pad128_align128"=>concurrent::<Atom128a128>(&args[2],threads,&args[4],all),"hot32_packed"=>{assert!(!all);concurrent::<AtomHot32>(&args[2],threads,&args[4],all)},"hot128_isolated"=>{assert!(!all);concurrent::<AtomHot128>(&args[2],threads,&args[4],all)},_=>panic!("unknown atomic layout")}},
        _=>panic!("--self-test | --serial variant n | --parallel variant threads pattern operation")
    }
}
