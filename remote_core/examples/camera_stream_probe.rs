//! Bounded camera capture/transport acceptance using the current authenticated
//! workspace client. No discovery identity, permissions, pixels or settings change.
//! This does not claim decode, compositor presentation, or physical display.
use protocol::session::{CameraFacing, CameraSwitchRequest, CaptureSourceInfo, SessionCommand, SubscriptionRequest};
use remote_core::{client_session::{SharedHostStats, VideoPacket}, stats::Statistics,
    workspace_session::{WorkspaceConnection, WorkspaceEvents}};
use std::{error::Error, net::SocketAddr, sync::Arc, time::{Duration, Instant}};
use tokio::sync::mpsc;
type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

async fn control(events: &mut WorkspaceEvents, accept: impl Fn(&SessionCommand)->bool) -> Result<SessionCommand> {
    let deadline=tokio::time::Instant::now()+Duration::from_secs(10);
    loop {
        let event=tokio::time::timeout_at(deadline,events.control.recv()).await?.ok_or("Workspace closed")?;
        if matches!(event,SessionCommand::Closed{..}) {return Err("Workspace closed".into());}
        if accept(&event) {return Ok(event);}
    }
}
async fn stop(conn: &WorkspaceConnection, events: &mut WorkspaceEvents, id: u32) -> Result<()> {
    conn.control(SessionCommand::Unsubscribe{id}).await?;
    control(events,|event|matches!(event,SessionCommand::Unsubscribed{id:got} if *got==id)).await?;
    Ok(())
}
async fn capture(conn: &WorkspaceConnection, events: &mut WorkspaceEvents, source: &CaptureSourceInfo, id: u32) -> Result<serde_json::Value> {
    let (tx,mut video)=mpsc::channel::<VideoPacket>(16);
    let start=Instant::now();
    conn.subscribe(SubscriptionRequest {id,source:source.source,width:1280,height:720,fps:30,bitrate_kbps:4000,audio:false},
        tx,Arc::new(Statistics::default()),Arc::new(SharedHostStats::default())).await?;
    let mut first=None;
    let mut sequence=None;
    let mut frames=0u64;
    let mut bytes=0u64;
    let timeout=tokio::time::sleep(Duration::from_secs(15));tokio::pin!(timeout);
    loop {
        tokio::select! {
            _=&mut timeout=>return Err("Camera did not deliver a bounded HEVC stream".into()),
            event=events.control.recv()=>match event {
                Some(SessionCommand::Error{request_id,reason}) if request_id==id=>return Err(reason.into()),
                Some(SessionCommand::Unsubscribed{id:got}) if got==id=>return Err("Camera sharing ended".into()),
                Some(SessionCommand::Closed{..})|None=>return Err("Workspace closed".into()),
                _=>{},
            },
            packet=video.recv()=>{
                let (packet,_,_)=packet.ok_or("Camera video route closed")?;
                if packet.header.ssrc!=id {return Err("Video crossed a subscription boundary".into());}
                first.get_or_insert_with(Instant::now);frames+=1;bytes+=packet.payload.len() as u64;
                if sequence.is_none() {sequence=remote_core::hevc_sequence::sequence_info(&packet.payload)?;}
                if first.unwrap().elapsed()>=Duration::from_secs(3) {break;}
            }
        }
    }
    let sequence=sequence.ok_or("Camera did not supply its HEVC sequence header")?;
    let duration=first.unwrap().elapsed().as_secs_f64();
    Ok(serde_json::json!({"source":source.source,"name":source.title,"camera":source.camera,
        "first_encoded_ms":first.unwrap().duration_since(start).as_secs_f64()*1000.,
        "frames":frames,"bytes":bytes,"fps":(frames.saturating_sub(1)) as f64/duration,
        "coded":sequence.coded,"visible":sequence.visible,"bit_depth":sequence.bit_depth,
        "signal":sequence.signal.map(|s|serde_json::json!({"full_range":s.full_range,"primaries":s.primaries,"transfer":s.transfer,"matrix":s.matrix})),
        "presentation":"not_evaluated"}))
}
async fn probe(conn: &WorkspaceConnection, events: &mut WorkspaceEvents) -> Result<serde_json::Value> {
    conn.control(SessionCommand::ListSources{request_id:1,known_icons:vec![]}).await?;
    let SessionCommand::Sources{sources,..}=control(events,|event|matches!(event,SessionCommand::Sources{request_id:1,..})).await? else{unreachable!()};
    let from=sources.iter().find(|s|s.camera.as_ref().is_some_and(|c|c.facing==CameraFacing::Back))
        .or_else(||sources.iter().find(|s|s.camera.is_some())).ok_or("No shared camera in this device's actual catalog")?;
    from.validate_camera()?;
    let mut captures=vec![capture(conn,events,from,256).await?];
    let target=sources.iter().find(|s|s.source!=from.source && s.camera.as_ref().is_some_and(|target|
        target.switch_group!=0 && from.camera.as_ref().is_some_and(|from|from.switch_group==target.switch_group)
            && target.facing==CameraFacing::Front));
    if let Some(target)=target {
        let request=CameraSwitchRequest{id:256,request_id:9,source_revision:0,from:from.source,target:target.source,delivery_request_id:None};
        conn.control(SessionCommand::RequestCameraSwitch(request.clone())).await?;
        let SessionCommand::CameraSwitchResult{ready,..}=control(events,|event|
            matches!(event,SessionCommand::CameraSwitchResult{request:got,..} if got==&request)).await? else{unreachable!()};
        ready?;
        stop(conn,events,256).await?;
        captures.push(capture(conn,events,target,512).await?);
        stop(conn,events,512).await?;
    } else {stop(conn,events,256).await?;}
    Ok(serde_json::json!({"ok":true,"authenticated_transport":true,"captures":captures,
        "permissions_changed":false,"presentation":"not_evaluated"}))
}
#[tokio::main]
async fn main() -> Result<()> {
    let mut args=std::env::args().skip(1);
    let endpoint:SocketAddr=args.next().ok_or("Use camera_stream_probe ENDPOINT with the existing paired profile")?.parse()?;
    if args.next().is_some() {return Err("Only one actual device endpoint is accepted".into());}
    let identity=remote_core::mesh::AppPrivateMeshConfigStore::new(remote_core::mesh::default_app_private_mesh_dir())
        .load()?.ok_or("An existing paired device profile is required")?;
    remote_core::session_crypto::use_paired_session_secret(identity.network_secret.expose_secret());
    if remote_core::session_crypto::paired_file_session_key().is_none() {
        return Err("Ambient session settings conflict with the paired profile".into());
    }
    let (conn,mut events)=tokio::time::timeout(Duration::from_secs(10),WorkspaceConnection::connect_media(endpoint)).await??;
    let result=probe(&conn,&mut events).await;
    conn.close().await;
    println!("{}",serde_json::to_string_pretty(&result?)?);
    Ok(())
}
