//! Pinned local speech models. Files are fetched directly, without archives.

use serde::Serialize;

pub(crate) const DEFAULT_MODEL_ID: &str = "parakeet-tdt-0.6b-v2-int8";
pub(crate) const DISK_MARGIN: u64 = 1024 * 1024 * 1024;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SpeechModel {
    pub id: &'static str,
    pub name: &'static str,
    pub family: &'static str,
    pub language: &'static str,
    pub repository: &'static str,
    #[serde(skip)]
    pub source_repository: &'static str,
    pub revision: &'static str,
    pub license: &'static str,
    pub recommended: bool,
    #[serde(skip)]
    pub directory: &'static str,
    #[serde(skip)]
    pub files: &'static [ModelFile],
}

#[derive(Debug)]
pub(crate) struct ModelFile {
    pub name: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
}

impl SpeechModel {
    pub(crate) fn size_bytes(&self) -> u64 {
        self.files.iter().map(|file| file.bytes).sum()
    }

    pub(crate) fn file_url(&self, file: &ModelFile) -> String {
        format!(
            "https://huggingface.co/{}/resolve/{}/{}",
            self.repository, self.revision, file.name
        )
    }

    pub(crate) fn attribution(&self) -> String {
        format!("{}\nCopyright NVIDIA Corporation.\nLicensed under CC-BY-4.0: https://creativecommons.org/licenses/by/4.0/\nSource: https://huggingface.co/{}\nONNX conversion and int8 quantization: https://huggingface.co/{}\nProvided without warranties; see the license.\n", self.name, self.source_repository, self.repository)
    }
}

pub(crate) fn model(id: &str) -> Result<&'static SpeechModel, String> {
    MODELS
        .iter()
        .find(|model| model.id == id)
        .ok_or_else(|| "Choose a speech model from the catalog".into())
}

pub(crate) static MODELS: &[SpeechModel] = &[
    SpeechModel {
        id: "parakeet-tdt-0.6b-v2-int8",
        name: "Parakeet TDT 0.6B v2 int8",
        family: "nemo_transducer",
        language: "English",
        repository: "csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8",
        source_repository: "nvidia/parakeet-tdt-0.6b-v2",
        revision: "1ab9323565ddb038682214b292f588070a538ce2",
        license: "CC-BY-4.0",
        recommended: cfg!(all(target_os = "macos", target_arch = "aarch64")),
        directory: "parakeet-tdt-0.6b-v2-en",
        files: &[
            ModelFile {
                name: "decoder.int8.onnx",
                bytes: 7257753,
                sha256: "b6bb64963457237b900e496ee9994b59294526439fbcc1fecf705b31a15c6b4e",
            },
            ModelFile {
                name: "encoder.int8.onnx",
                bytes: 652184296,
                sha256: "a32b12d17bbbc309d0686fbbcc2987b5e9b8333a7da83fa6b089f0a2acd651ab",
            },
            ModelFile {
                name: "joiner.int8.onnx",
                bytes: 1739080,
                sha256: "7946164367946e7f9f29a122407c3252b680dbae9a51343eb2488d057c3c43d2",
            },
            ModelFile {
                name: "tokens.txt",
                bytes: 9384,
                sha256: "ec182b70dd42113aff6c5372c75cac58c952443eb22322f57bbd7f53977d497d",
            },
        ],
    },
    SpeechModel {
        id: "parakeet-tdt-0.6b-v3-int8",
        name: "Parakeet TDT 0.6B v3 int8",
        family: "nemo_transducer",
        language: "25 languages",
        repository: "csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8",
        source_repository: "nvidia/parakeet-tdt-0.6b-v3",
        revision: "2bda32ec70b097a55adaa07d9a7173915b43cc78",
        license: "CC-BY-4.0",
        recommended: false,
        directory: "parakeet-tdt-0.6b-v3",
        files: &[
            ModelFile {
                name: "decoder.int8.onnx",
                bytes: 11845275,
                sha256: "179e50c43d1a9de79c8a24149a2f9bac6eb5981823f2a2ed88d655b24248db4e",
            },
            ModelFile {
                name: "encoder.int8.onnx",
                bytes: 652184281,
                sha256: "acfc2b4456377e15d04f0243af540b7fe7c992f8d898d751cf134c3a55fd2247",
            },
            ModelFile {
                name: "joiner.int8.onnx",
                bytes: 6355277,
                sha256: "3164c13fc2821009440d20fcb5fdc78bff28b4db2f8d0f0b329101719c0948b3",
            },
            ModelFile {
                name: "tokens.txt",
                bytes: 93939,
                sha256: "d58544679ea4bc6ac563d1f545eb7d474bd6cfa467f0a6e2c1dc1c7d37e3c35d",
            },
        ],
    },
];
