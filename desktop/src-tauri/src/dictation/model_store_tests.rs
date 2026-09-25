use super::*;

static MODEL: SpeechModel = SpeechModel {
    id: "test",
    name: "Test",
    family: "nemo_transducer",
    language: "English",
    repository: "test/model",
    revision: "test-revision",
    license: "CC-BY-4.0",
    recommended: false,
    directory: "test-model",
    files: &[ModelFile {
        name: "model.onnx",
        bytes: 3,
        sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    }],
};

async fn server(body: &'static str) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route("/model", axum::routing::get(move || async move { body })),
        )
        .await
        .unwrap();
    });
    (format!("http://{address}/model"), handle)
}

#[tokio::test]
async fn download_installs_verified_files_and_remove_clears_readiness() {
    let root = tempfile::tempdir().unwrap();
    let (url, task) = server("abc").await;
    install(
        root.path(),
        &MODEL,
        &reqwest::Client::new(),
        Some(DISK_MARGIN + 3),
        |_| url.clone(),
        |_| {},
    )
    .await
    .unwrap();
    let ready = ready_path(root.path(), &MODEL).expect("verified install is ready");
    assert_eq!(std::fs::read(ready.join("model.onnx")).unwrap(), b"abc");
    assert!(ready.join(LICENSE).is_file());
    remove_from(root.path(), &MODEL).unwrap();
    assert!(
        ready_path(root.path(), &MODEL).is_none(),
        "removal clears readiness"
    );
    task.abort();
}

#[tokio::test]
async fn corrupt_download_never_replaces_existing_files() {
    let root = tempfile::tempdir().unwrap();
    let old = root.path().join(MODEL.directory);
    std::fs::create_dir(&old).unwrap();
    std::fs::write(old.join("keep"), b"old").unwrap();
    let (url, task) = server("bad").await;
    let error = install(
        root.path(),
        &MODEL,
        &reqwest::Client::new(),
        Some(DISK_MARGIN + 3),
        |_| url.clone(),
        |_| {},
    )
    .await
    .unwrap_err();
    assert!(error.contains("verification failed"), "{error}");
    assert_eq!(std::fs::read(old.join("keep")).unwrap(), b"old");
    assert!(ready_path(root.path(), &MODEL).is_none());
    assert_eq!(
        std::fs::read_dir(root.path()).unwrap().count(),
        1,
        "failed staging is removed"
    );
    task.abort();
}

#[tokio::test]
async fn download_refuses_low_or_unknown_space_before_network_or_files() {
    let root = tempfile::tempdir().unwrap();
    for free in [None, Some(DISK_MARGIN + 2)] {
        let error = install(
            root.path(),
            &MODEL,
            &reqwest::Client::new(),
            free,
            |_| panic!("must refuse before network"),
            |_| {},
        )
        .await
        .unwrap_err();
        assert!(error.contains("space"), "{error}");
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }
}

#[test]
fn catalog_ids_are_pinned_and_unknown_models_are_refused() {
    assert!(catalog::model("../../other").is_err());
    assert_eq!(catalog::MODELS.len(), 2);
    for model in catalog::MODELS {
        assert_eq!(model.revision.len(), 40);
        assert_eq!(model.family, "nemo_transducer");
        assert!(model
            .files
            .iter()
            .all(|file| file.sha256.len() == 64 && file.bytes > 0));
    }
}
