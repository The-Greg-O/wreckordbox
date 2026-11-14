#[path="../../wreckordbox-sidecar/tests/fingerprint_support/mod.rs"] mod fixture;
use wreckordbox_sidecar::{fingerprint_file, audio_info, Sidecar, Source, SourceKind,NewFact,Kind,stems::{NewStems,Role}};
fn main() {
    let root=std::path::PathBuf::from(std::env::args().nth(1).expect("output directory"));std::fs::create_dir_all(&root).unwrap();
    let sidecar=Sidecar::open(&root.join("sidecar")).unwrap();
    let mut golden=serde_json::Map::new();
    for format in fixture::FORMATS {
        let audio=fixture::write(&root,format);let fp=fingerprint_file(&audio).unwrap();let frames=if format=="m4a" {88200} else {let info=audio_info(&audio).unwrap();(info.samples*44100/u64::from(info.sample_rate)) as u32};
        let source=Source::new(SourceKind::Tool,"separation-engine");let run=sidecar.begin_run(&source,"Foundation synthetic Stems fixture").unwrap();
        let files:Vec<_>=Role::ALL.into_iter().map(|role|{let p=root.join(format!("{format}-{role}.wav"));std::fs::write(&p,fixture::wav(frames)).unwrap();(role,p)}).collect();
        let stems=sidecar.record_stems(&NewStems{track_id:format,fingerprint:&fp,parent_samples:u64::from(frames),sample_rate:44100,files:&files,source:&source,produced_by:serde_json::json!({"fixture":"foundation ec1d477d","engine_version":"synthetic"}),run}).unwrap();sidecar.finish_run(run,"done").unwrap();
        let source=Source::new(SourceKind::Tool,wreckordbox_intensity::METHOD_NAME);
        let run=sidecar.begin_run(&source,"Foundation measured Intensity fixture").unwrap();
        let intensity=wreckordbox_intensity::measure_file(&audio,&[],&||false).unwrap().unwrap();
        sidecar.record(&NewFact{track_id:format,fingerprint:Some(&fp),kind:Kind::measurement("intensity"),value:serde_json::to_value(intensity).unwrap(),source:&source,confidence:1.0,run}).unwrap();sidecar.finish_run(run,"done").unwrap();
        golden.insert(format.into(),serde_json::json!({"a1":fp,"stem_frames":frames,"stem_fact":stems.fact_id}));
        let mut trailed=std::fs::read(&audio).unwrap();trailed.extend(fixture::ape());
        let trailer_path=root.join(format!("{format}-ape.{}",fixture::extension(format)));
        std::fs::write(&trailer_path,trailed).unwrap();
        golden.get_mut(format).unwrap()["ape_legacy"]=serde_json::json!(fingerprint_file(&trailer_path).unwrap());
        if format=="aiff" {
          let keyed=root.join("keyed.aiff");std::fs::copy(&audio,&keyed).unwrap();
          let rbl_db::key_tag::prepared::Outcome::Ready(mut pending)=rbl_db::key_tag::prepared::prepare(&keyed,None,Some("Am")).unwrap() else {panic!("key writer skipped AIFF")};
          pending.publish().unwrap();drop(pending);
          golden.get_mut(format).unwrap()["key_legacy"]=serde_json::json!(fingerprint_file(&keyed).unwrap());
        }

    }
    std::fs::write(root.join("foundation.json"),serde_json::to_vec_pretty(&golden).unwrap()).unwrap();
    println!("{}",serde_json::to_string_pretty(&golden).unwrap());
}
