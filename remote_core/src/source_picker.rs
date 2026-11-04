//! Shared directory presentation order. No process ancestry is inferred.
use protocol::session::{CaptureSource, CaptureSourceInfo};

pub fn application_group(source:&CaptureSourceInfo)->&str {
    let app=source.application.trim();
    if app.is_empty(){"Other applications"}else{app}
}

pub fn sort(sources:&mut [CaptureSourceInfo]) {
    sources.sort_by_cached_key(|source| {
        let (category,id)=match source.source {
            CaptureSource::MainDisplay=>(0,0),CaptureSource::Display(id)=>(0,u64::from(id)),
            CaptureSource::Window(id)=>(1,u64::from(id)),CaptureSource::Camera(id)=>(2,id),
        };
        let app=if category==1 {source.application.trim().to_lowercase()}else{String::new()};
        (category,category==1&&app.is_empty(),app,source.title.trim().to_lowercase(),id)
    });
}

/// The caller normalizes the query once per filter update/render.
pub fn matches_name(source:&CaptureSourceInfo,normalized_query:&str)->bool {
    normalized_query.is_empty() || source.title.to_lowercase().contains(normalized_query)
        || source.application.to_lowercase().contains(normalized_query)
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::session::SourceCategory;
    fn window(id:u32,app:&str,title:&str,pid:i32)->CaptureSourceInfo {
        CaptureSourceInfo {source:CaptureSource::Window(id),application:app.into(),title:title.into(),process_id:Some(pid),
            width:640,height:480,supports_input:true,camera:None}
    }
    #[test]
    fn grouping_and_duplicate_titles_are_stable_without_claiming_parent_processes() {
        let mut sources=vec![window(9,"Zeta","A",10),window(5," editor ","Notes",20),
            window(2,"Editor","Notes",30),window(7,"Editor","Browser",40),window(3,"","Unknown",50)];
        sort(&mut sources);
        assert_eq!(sources.iter().map(|s|s.source).collect::<Vec<_>>(),vec![CaptureSource::Window(7),
            CaptureSource::Window(2),CaptureSource::Window(5),CaptureSource::Window(9),CaptureSource::Window(3)]);
        let expected=sources.clone();sources.reverse();sort(&mut sources);assert_eq!(sources,expected);
        assert_eq!(application_group(&sources[4]),"Other applications");
        assert!(sources.iter().all(|s|s.source.category()==SourceCategory::Window));
    }
    #[test]
    fn names_match_window_or_application_with_unicode_and_case_normalized_queries() {
        let source=window(1,"Editor","中文 Notes 🙂",20);
        for query in ["editor","notes","中文","🙂",""] {assert!(matches_name(&source,query));}
        assert!(!matches_name(&source,"terminal"));
    }
}
