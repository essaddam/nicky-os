use super::*;
use crate::{Transport, Version, protocol};
use std::fmt::Write as _;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU8, Ordering},
};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::task::{JoinHandle, JoinSet};

pub(crate) struct Server {
    pub port: u16,
    pub actions: Arc<Mutex<Vec<String>>>,
    pub content_types: Arc<Mutex<Vec<String>>>,
    pub mode: Arc<AtomicU8>,
    task: JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Server {
    pub async fn start(status: u16, body: Vec<u8>, delay: std::time::Duration) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let actions = Arc::new(Mutex::new(vec![]));
        let seen = actions.clone();
        let content_types = Arc::new(Mutex::new(Vec::new()));
        let types = content_types.clone();
        let mode = Arc::new(AtomicU8::new(0));
        let control = mode.clone();
        let task = tokio::spawn(async move {
            let mut tasks = JoinSet::new();
            loop {
                tokio::select! {
                    accepted=listener.accept()=>{
                        let (stream,_)=accepted.unwrap();let (body,seen,mode,types)=(body.clone(),seen.clone(),control.clone(),types.clone());
                        tasks.spawn(async move{
                            let (read,mut write)=stream.into_split();let mut reader=BufReader::new(read);let mut first=String::new();reader.read_line(&mut first).await.unwrap();
                            assert_eq!(first.trim(),"POST /soap/server_sa/ HTTP/1.1");
                            let mut len=0;let mut action=String::new();loop{let mut line=String::new();reader.read_line(&mut line).await.unwrap();if line=="\r\n"{break;}let (key,value)=line.split_once(':').unwrap();if key.eq_ignore_ascii_case("content-length"){len=value.trim().parse().unwrap();}else if key.eq_ignore_ascii_case("content-type"){types.lock().unwrap().push(value.trim().to_owned());}else if key.eq_ignore_ascii_case("soapaction"){action=value.trim().into();}}
                            assert!(len<16384);let mut data=vec![0;len];reader.read_exact(&mut data).await.unwrap();seen.lock().unwrap().push(action.clone());
                            tokio::time::sleep(if mode.load(Ordering::SeqCst)==4{std::time::Duration::from_secs(30)}else{delay}).await;
                            let name=action.rsplit('#').next().unwrap_or("").to_owned();
                            let body=if status==0 {match name.as_str(){
                                "SOAPLogin"=>protocol::response(Action::LoginV2,"",0).into_bytes(),
                                "Authenticate"=>protocol::response(Action::LoginV1,"",0).into_bytes(),
                                "GetInfo"=>include_bytes!("../../../integrations/netgear/fixtures/info.xml").to_vec(),
                                "GetSystemInfo"=>include_bytes!("../../../integrations/netgear/fixtures/system.xml").to_vec(),
                                "GetEthernetLinkStatus"=>include_bytes!("../../../integrations/netgear/fixtures/ethernet.xml").to_vec(),
                                "GetTrafficMeterStatistics"=>include_bytes!("../../../integrations/netgear/fixtures/traffic.xml").to_vec(),
                                "GetAttachDevice2"=>include_bytes!("../../../integrations/netgear/fixtures/clients-v2.xml").to_vec(),
                                "GetAttachDevice"=>include_bytes!("../../../integrations/netgear/fixtures/clients-v1.xml").to_vec(),
                                _=>panic!("forbidden action"),
                            }}else{body};
                            let body=if mode.load(Ordering::SeqCst)==1 && name=="GetAttachDevice2" {String::from_utf8(body).unwrap().replace("ee:02","ee:01").into_bytes()}else if mode.load(Ordering::SeqCst)==2 && name=="GetInfo" {String::from_utf8(body).unwrap().replace("synthetic-router-01","changed-router").into_bytes()}else{body};
                            let body=if mode.load(Ordering::SeqCst)==3 && name=="GetAttachDevice2" {
                                let mut devices=String::new(); for i in 0..32 { write!(&mut devices,"<Device><MAC>aa:bb:cc:dd:ee:{i:02x}</MAC><ConnectionType>5GHz</ConnectionType><SignalStrength>75</SignalStrength><Linkspeed>1000 Mbit/s</Linkspeed></Device>").unwrap(); }
                                protocol::response(Action::ClientsV2,&format!("<NewAttachDevice>{devices}</NewAttachDevice>"),0).into_bytes()
                            }else{body};
                            let status=if mode.load(Ordering::SeqCst)==5 && name=="GetSystemInfo" {423} else if status==0{200}else{status};
                            let cookie=format!("sid={:x}",std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos());
                            let header=format!("HTTP/1.1 {status} result\r\nContent-Length: {}\r\nConnection: close\r\nSet-Cookie: {cookie}; Path=/\r\nLocation: http://192.168.1.9/forbidden\r\n\r\n",body.len());
                            let _=write.write_all(header.as_bytes()).await;let _=write.write_all(&body).await;
                        });
                    },
                    Some(result)=tasks.join_next()=>result.unwrap(),
                }
            }
        });
        Self {
            port,
            actions,
            content_types,
            mode,
            task,
        }
    }
    pub fn config(&self) -> Config {
        Config {
            host: "127.0.0.1".into(),
            port: self.port,
            transport: Transport::Http,
            login: Version::V2,
            inventory: Version::V2,
            allow_plain_auth: true,
            every: 60,
            inventory_every: 300,
            info_every: 3600,
            selected: vec![],
        }
    }
}
#[tokio::test]
async fn real_http_uses_only_the_fixed_action_and_rejects_large_answers() {
    let server = Server::start(
        200,
        protocol::response(Action::Info, "<SerialNumber>test-router</SerialNumber>", 0)
            .into_bytes(),
        std::time::Duration::ZERO,
    )
    .await;
    let wire = Network {
        config: server.config(),
        tls: None,
    };
    let result = wire
        .send(
            Action::Info,
            protocol::envelope(Action::Info, None, "test-session"),
            None,
        )
        .await
        .unwrap();
    assert!(protocol::decode(&result.body, Action::Info).is_ok());
    assert_eq!(
        *server.content_types.lock().unwrap(),
        vec!["multipart/form-data"]
    );
    assert_eq!(
        *server.actions.lock().unwrap(),
        vec!["urn:NETGEAR-ROUTER:service:DeviceInfo:1#GetInfo"]
    );
    let server = Server::start(
        200,
        vec![b'x'; crate::xml::MAX_BODY + 1],
        std::time::Duration::ZERO,
    )
    .await;
    let wire = Network {
        config: server.config(),
        tls: None,
    };
    assert!(
        wire.send(
            Action::Info,
            protocol::envelope(Action::Info, None, "test-session"),
            None
        )
        .await
        .is_err()
    );
}
#[tokio::test]
async fn slow_exchange_has_a_global_deadline() {
    let server = Server::start(200, vec![], std::time::Duration::from_secs(30)).await;
    let wire = Network {
        config: server.config(),
        tls: None,
    };
    let start = tokio::time::Instant::now();
    assert!(
        wire.send(
            Action::Info,
            protocol::envelope(Action::Info, None, "test-session"),
            None
        )
        .await
        .is_err()
    );
    assert!(start.elapsed() < std::time::Duration::from_secs(7));
}
