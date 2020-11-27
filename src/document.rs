// Copyright 2020 the Tectonic Project
// Licensed under the MIT License.

//! Tectonic document definitions.

use reqwest::Url;
use std::{
    collections::HashMap,
    env, fs,
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
};

use crate::{
    config, ctry,
    driver::{OutputFormat, PassSetting, ProcessingSessionBuilder},
    errors::{ErrorKind, Result},
    io::{
        cached_itarbundle::{resolve_url, CachedITarBundle},
        dirbundle::DirBundle,
        zipbundle::ZipBundle,
        Bundle,
    },
    status::StatusBackend,
    tt_error,
    workspace::WorkspaceCreator,
};

/// A Tectonic document.
#[derive(Debug)]
pub struct Document {
    /// The directory containing the `Tectonic.toml` file and document source.
    src_dir: PathBuf,

    /// The directory where document build artifacts will be output. By default
    /// this will be a subdirectory of `src_dir` named `build`.
    build_dir: PathBuf,

    /// The document name. This will be used to name build artifacts and the
    /// like, and so should be relatively filesystem-friendly. It does not
    /// need to be the same as the document title.
    name: String,

    /// The name of core TeX file bundle upon which this document is based.
    /// Either a URL or a local path.
    bundle_loc: String,

    /// The different outputs that are created from the document source. These
    /// may have different formats (e.g., PDF and HTML) or the same format but
    /// different settings (e.g., PDF with A4 paper and PDF with US Letter
    /// paper).
    outputs: HashMap<String, OutputProfile>,
}

fn default_outputs() -> HashMap<String, OutputProfile> {
    let mut outputs = HashMap::new();
    outputs.insert(
        "default".to_owned(),
        OutputProfile {
            name: "default".to_owned(),
            target_type: BuildTargetType::Pdf,
            format: "latex".to_owned(),
        },
    );
    outputs
}

impl Document {
    /// Initialize a Document based on a TOML specification
    pub(crate) fn new_from_toml<R: Read>(
        src_dir: PathBuf,
        build_dir: PathBuf,
        toml_data: &mut R,
    ) -> Result<Self> {
        let mut toml_text = String::new();
        toml_data.read_to_string(&mut toml_text)?;
        let doc: syntax::Document = toml::from_str(&toml_text)?;

        // TODO: serialize outputs in the TOML
        let outputs = default_outputs();

        Ok(Document {
            src_dir,
            build_dir,
            name: doc.doc.name,
            bundle_loc: doc.doc.bundle,
            outputs,
        })
    }

    /// Create a new in-memory Document, based on the settings of a
    /// WorkspaceCreator object.
    pub(crate) fn new_for_creator(
        wc: &WorkspaceCreator,
        config: &config::PersistentConfig,
        status: &mut dyn StatusBackend,
    ) -> Result<Self> {
        let src_dir = wc.root_dir().to_owned();

        let mut build_dir = src_dir.clone();
        build_dir.push("build");

        // We're a bit roundabout in how we figure out the name of the
        // containing src_dir, in an effort to Do The Right Thing with symlinks
        // and whatnot.
        let name = {
            let mut name = "document".to_owned();
            let mut tried_src_path = false;

            if let Some(c) = src_dir.components().next_back() {
                if let Component::Normal(t) = c {
                    tried_src_path = true;

                    if let Some(s) = t.to_str() {
                        name = s.to_owned();
                    }
                }
            }

            if !tried_src_path {
                if let Ok(cwd) = env::current_dir() {
                    let full_path = cwd.join(&src_dir);

                    if let Some(c) = full_path.components().next_back() {
                        if let Component::Normal(t) = c {
                            if let Some(s) = t.to_str() {
                                name = s.to_owned();
                            }
                        }
                    }
                }
            }

            name
        };

        // Determine the bundle URL that we'll put in as the default.

        let bundle_loc = resolve_url(config.default_bundle_loc(), status)?;

        // All done.
        Ok(Document {
            src_dir,
            build_dir,
            name,
            bundle_loc,
            outputs: default_outputs(),
        })
    }

    /// Write out this document's state as a new TOML file. This should only be
    /// used when creating a totally new document; otherwise TOML rewriting
    /// should be used.
    pub(crate) fn create_toml(&self) -> Result<()> {
        let doc = syntax::Document {
            doc: syntax::DocSection {
                name: self.name.clone(),
                bundle: self.bundle_loc.clone(),
            },
        };

        let toml_text = toml::to_string_pretty(&doc)?;

        let mut toml_path = self.src_dir.clone();
        toml_path.push("Tectonic.toml");

        let mut toml_file = ctry!(fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&toml_path);
            "couldn\'t create `{}`", toml_path.display()
        );

        toml_file.write_all(toml_text.as_bytes())?;
        Ok(())
    }
}

/// Persistent settings for a document build.
#[derive(Clone, Debug)]
pub struct OutputProfile {
    name: String,
    target_type: BuildTargetType,
    format: String,
}

/// The output target type of a document build.
#[derive(Clone, Debug)]
pub enum BuildTargetType {
    /// Output to the Portable Document Format (PDF).
    Pdf,
}

/// Temporary options for a document build.
#[derive(Clone, Debug)]
pub struct BuildOptions {}

const DEFAULT_PRIMARY_INPUT: &[u8] = br#"
\input _preamble.tex
\input index.tex
\input _postamble.tex
"#;

impl Document {
    /// Iterate over the names of the output profiles defined for this document.
    /// These may have different formats (e.g., PDF and HTML) or the same format
    /// but different settings (e.g., PDF with A4 paper and PDF with US Letter
    /// paper).
    pub fn output_names(&self) -> impl Iterator<Item = &str> {
        self.outputs.keys().map(|k| k.as_ref())
    }

    /// Get default the build options associated with an output profile.
    ///
    /// Panics if the output name is not one of the ones associated with this
    /// document.
    pub fn build_options_for(&self, _output_profile: &str) -> BuildOptions {
        BuildOptions {}
    }

    /// Get the bundle used by this document.
    pub fn bundle(&self, status: &mut dyn StatusBackend) -> Result<Box<dyn Bundle>> {
        fn bundle_from_path(p: PathBuf) -> Result<Box<dyn Bundle>> {
            if p.is_dir() {
                Ok(Box::new(DirBundle::new(p)))
            } else {
                Ok(Box::new(ZipBundle::open(p)?))
            }
        }

        if let Ok(url) = Url::parse(&self.bundle_loc) {
            if url.scheme() != "file" {
                let bundle = CachedITarBundle::new(&self.bundle_loc, false, None, status)?;
                Ok(Box::new(bundle))
            } else {
                let file_path = url.to_file_path().map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidInput, "failed to parse local path")
                })?;
                bundle_from_path(file_path)
            }
        } else {
            bundle_from_path(Path::new(&self.bundle_loc).to_owned())
        }
    }

    /// Build the document.
    pub fn build(
        &self,
        output_profile: &str,
        _options: &BuildOptions,
        status: &mut dyn StatusBackend,
    ) -> Result<i32> {
        let profile = self.outputs.get(output_profile).unwrap();

        let output_format = match profile.target_type {
            BuildTargetType::Pdf => OutputFormat::Pdf,
        };

        let mut sess_builder = ProcessingSessionBuilder::default();
        sess_builder.output_format(output_format);
        sess_builder.format_name(&profile.format);
        sess_builder.pass(PassSetting::Default);
        sess_builder.primary_input_buffer(DEFAULT_PRIMARY_INPUT);
        sess_builder.tex_input_name(output_profile);
        sess_builder.bundle(self.bundle(status)?);

        let mut tex_dir = self.src_dir.clone();
        tex_dir.push("src");
        sess_builder.filesystem_root(&tex_dir);

        let mut output_dir = self.build_dir.clone();
        output_dir.push(output_profile);
        ctry!(
            fs::create_dir_all(&output_dir);
            "couldn\'t create output directory `{}`", output_dir.display()
        );
        sess_builder.output_dir(&output_dir);

        let mut sess = sess_builder.create(status)?;
        let result = sess.run(status);

        if let Err(e) = &result {
            if let ErrorKind::EngineError(engine) = e.kind() {
                if let Some(output) = sess.io.mem.files.borrow().get(sess.io.mem.stdout_key()) {
                    tt_error!(
                        status,
                        "something bad happened inside {}; its output follows:\n",
                        engine
                    );

                    status.dump_error_logs(&output.data);
                }
            }
        }

        result.map(|_| 0)
    }
}

/// The concrete syntax for saving document state, wired up via serde.
mod syntax {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Deserialize, Serialize)]
    pub struct Document {
        pub doc: DocSection,
    }

    #[derive(Debug, Deserialize, Serialize)]
    pub struct DocSection {
        pub name: String,
        pub bundle: String,
    }
}
