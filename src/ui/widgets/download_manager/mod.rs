pub(crate) mod asset;
pub(crate) mod docker;
mod download_item;

use crate::ui::widgets::download_manager::asset::Asset;
use crate::ui::widgets::download_manager::docker::Docker;
use crate::ui::widgets::download_manager::download_item::EpicDownloadItem;
use glib::clone;
use gtk4::subclass::prelude::*;
use gtk4::{self, gio, glib, prelude::*, CompositeTemplate};
use gtk_macros::action;
use log::{debug, error, info};
use reqwest::Url;
use std::ffi::OsStr;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum DownloadMsg {
    ProcessItemThumbnail(String, Vec<u8>),
    StartAssetDownload(
        String,
        Vec<egs_api::api::types::download_manifest::DownloadManifest>,
    ),
    PerformAssetDownload(
        String,
        String,
        String,
        egs_api::api::types::download_manifest::FileManifestList,
    ),
    PerformChunkDownload(Url, PathBuf, String),
    RedownloadChunk(Url, PathBuf, String),
    PauseChunk(Url, PathBuf, String),
    CancelChunk(Url, PathBuf, String),
    ChunkDownloadProgress(String, u128, bool),
    FinalizeFileDownload(String, asset::DownloadedFile),
    FileAlreadyDownloaded(String, u128, String, String),
    FileExtracted(String),
    PerformDockerEngineDownload(String, u64, Vec<(String, u64)>),
    DockerDownloadProgress(String, u64),
    DockerBlobFinished(String, String),
    DockerBlobFailed(String, (String, u64)),
    DockerExtractionFinished(String),
    DockerCanceled(String, (String, u64)),
    DockerPaused(String, (String, u64)),
    IOError(String),
}

#[derive(Debug, Clone)]
pub enum DownloadStatus {
    Init,
    Downloaded,
    Extracting,
    Extracted,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PostDownloadAction {
    Copy(String, bool),
    NoVault,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ThreadMessages {
    Cancel,
    Pause,
}

pub(crate) mod imp {
    use super::*;
    use crate::window::EpicAssetManagerWindow;
    use gtk4::gio;
    use gtk4::glib::{ParamSpec, ParamSpecBoolean};
    use once_cell::sync::OnceCell;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use threadpool::ThreadPool;

    #[derive(Debug, CompositeTemplate)]
    #[template(resource = "/io/github/achetagames/epic_asset_manager/download_manager.ui")]
    pub struct EpicDownloadManager {
        pub actions: gio::SimpleActionGroup,
        pub settings: gio::Settings,
        pub window: OnceCell<EpicAssetManagerWindow>,
        pub download_pool: ThreadPool,
        pub thumbnail_pool: ThreadPool,
        pub image_pool: ThreadPool,
        pub file_pool: ThreadPool,
        pub sender: gtk4::glib::Sender<super::DownloadMsg>,
        pub receiver: RefCell<Option<gtk4::glib::Receiver<super::DownloadMsg>>>,
        pub download_items: RefCell<
            HashMap<String, crate::ui::widgets::download_manager::download_item::EpicDownloadItem>,
        >,
        pub downloaded_files: RefCell<HashMap<String, super::asset::DownloadedFile>>,
        pub downloaded_chunks: RefCell<HashMap<String, Vec<String>>>,
        pub asset_guids: RefCell<HashMap<String, Vec<String>>>,
        pub paused_asset_chunks: RefCell<HashMap<String, Vec<(Url, PathBuf)>>>,
        pub paused_docker_digests: RefCell<HashMap<String, Vec<(String, u64)>>>,
        pub thread_senders: RefCell<HashMap<String, Vec<std::sync::mpsc::Sender<ThreadMessages>>>>,
        pub chunk_urls: RefCell<HashMap<String, Vec<Url>>>,
        pub docker_digests: RefCell<HashMap<String, Vec<(String, DownloadStatus)>>>,
        #[template_child]
        pub downloads: TemplateChild<gtk4::ListBox>,
        has_children: RefCell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for EpicDownloadManager {
        const NAME: &'static str = "EpicDownloadManager";
        type Type = super::EpicDownloadManager;
        type ParentType = gtk4::Box;

        fn new() -> Self {
            let (sender, receiver) = gtk4::glib::MainContext::channel(gtk4::glib::PRIORITY_DEFAULT);
            Self {
                actions: gio::SimpleActionGroup::new(),
                settings: gio::Settings::new(crate::config::APP_ID),
                window: OnceCell::new(),
                sender,
                download_pool: ThreadPool::with_name("Download Pool".to_string(), 5),
                receiver: RefCell::new(Some(receiver)),
                download_items: RefCell::new(HashMap::new()),
                downloaded_files: RefCell::new(HashMap::new()),
                downloaded_chunks: RefCell::new(HashMap::new()),
                asset_guids: RefCell::new(HashMap::new()),
                paused_asset_chunks: RefCell::new(HashMap::new()),
                paused_docker_digests: RefCell::new(HashMap::new()),
                thread_senders: RefCell::new(HashMap::new()),
                chunk_urls: RefCell::new(HashMap::new()),
                docker_digests: RefCell::new(HashMap::new()),
                downloads: TemplateChild::default(),
                thumbnail_pool: ThreadPool::with_name("Thumbnail Pool".to_string(), 5),
                image_pool: ThreadPool::with_name("Image Pool".to_string(), 5),
                file_pool: ThreadPool::with_name("File Pool".to_string(), 1),
                has_children: RefCell::new(false),
            }
        }

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        // You must call `Widget`'s `init_template()` within `instance_init()`.
        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for EpicDownloadManager {
        fn constructed(&self, obj: &Self::Type) {
            self.parent_constructed(obj);
            obj.setup_actions();
            obj.setup_messaging();
        }

        fn signals() -> &'static [gtk4::glib::subclass::Signal] {
            static SIGNALS: once_cell::sync::Lazy<Vec<gtk4::glib::subclass::Signal>> =
                once_cell::sync::Lazy::new(|| {
                    vec![gtk4::glib::subclass::Signal::builder(
                        "tick",
                        &[],
                        <()>::static_type().into(),
                    )
                    .flags(glib::SignalFlags::ACTION)
                    .build()]
                });
            SIGNALS.as_ref()
        }

        fn properties() -> &'static [ParamSpec] {
            use once_cell::sync::Lazy;
            static PROPERTIES: Lazy<Vec<ParamSpec>> = Lazy::new(|| {
                vec![ParamSpecBoolean::new(
                    "has-items",
                    "has items",
                    "Has Items",
                    false,
                    glib::ParamFlags::READWRITE,
                )]
            });
            PROPERTIES.as_ref()
        }

        fn set_property(
            &self,
            _obj: &Self::Type,
            _id: usize,
            value: &glib::Value,
            pspec: &ParamSpec,
        ) {
            match pspec.name() {
                "has-items" => {
                    let has_children = value.get().unwrap();
                    self.has_children.replace(has_children);
                }
                _ => unimplemented!(),
            }
        }

        fn property(&self, _obj: &Self::Type, _id: usize, pspec: &ParamSpec) -> glib::Value {
            match pspec.name() {
                "has-items" => self.has_children.borrow().to_value(),
                _ => unimplemented!(),
            }
        }
    }

    impl WidgetImpl for EpicDownloadManager {}
    impl BoxImpl for EpicDownloadManager {}
}

glib::wrapper! {
    pub struct EpicDownloadManager(ObjectSubclass<imp::EpicDownloadManager>)
        @extends gtk4::Widget, gtk4::Box;
}

impl Default for EpicDownloadManager {
    fn default() -> Self {
        Self::new()
    }
}

impl EpicDownloadManager {
    pub fn new() -> Self {
        let stack: Self = glib::Object::new(&[]).expect("Failed to create EpicDownloadManager");

        stack
    }

    pub fn set_window(&self, window: &crate::window::EpicAssetManagerWindow) {
        let self_ = self.imp();
        // Do not run this twice
        if self_.window.get().is_some() {
            return;
        }

        self_.window.set(window.clone()).unwrap();
    }

    pub fn setup_actions(&self) {
        let self_ = self.imp();

        action!(
            self_.actions,
            "close",
            clone!(@weak self as details => move |_, _| {
                let self_: &imp::EpicDownloadManager = imp::EpicDownloadManager::from_instance(&details);
                if let Some(w) = self_.window.get() {
                   w.show_logged_in();
                }
            })
        );

        self.insert_action_group("download_manager", Some(&self_.actions));
    }

    pub fn setup_messaging(&self) {
        let self_ = self.imp();
        let receiver = self_.receiver.borrow_mut().take().unwrap();
        receiver.attach(
            None,
            clone!(@weak self as download_manager => @default-panic, move |msg| {
                download_manager.update(msg);
                glib::Continue(true)
            }),
        );
    }

    pub fn update(&self, msg: DownloadMsg) {
        let self_ = self.imp();
        match msg {
            DownloadMsg::ProcessItemThumbnail(id, image) => {
                let item = match self.get_item(&id) {
                    None => return,
                    Some(i) => i,
                };
                let pixbuf_loader = gtk4::gdk_pixbuf::PixbufLoader::new();
                pixbuf_loader.write(&image).unwrap();
                pixbuf_loader.close().ok();

                if let Some(pix) = pixbuf_loader.pixbuf() {
                    item.set_property("thumbnail", &pix);
                };
            }
            DownloadMsg::StartAssetDownload(id, manifest) => {
                self.start_download_asset(&id, &manifest);
            }
            DownloadMsg::PerformAssetDownload(id, release, name, manifest) => {
                self.download_asset_file(id, release, name, manifest);
            }
            DownloadMsg::PerformChunkDownload(link, path, guid) => {
                self.download_chunk(link, path, guid);
            }
            DownloadMsg::RedownloadChunk(link, path, guid) => {
                self.redownload_chunk(&link, path, &guid);
            }
            DownloadMsg::ChunkDownloadProgress(guid, size, finished) => {
                self.chunk_progress_report(&guid, size, finished);
            }
            DownloadMsg::FinalizeFileDownload(file, file_details) => {
                self.finalize_file_download(&file, file_details);
            }
            DownloadMsg::FileAlreadyDownloaded(id, progress, fullname, filename) => {
                self.file_already_extracted(id, progress, fullname, filename);
            }
            DownloadMsg::FileExtracted(id) => {
                let item = match self.get_item(&id) {
                    None => {
                        return;
                    }
                    Some(i) => i,
                };
                item.file_processed();
                self.emit_by_name::<()>("tick", &[]);
            }
            DownloadMsg::PerformDockerEngineDownload(version, size, digests) => {
                self.perform_docker_blob_downloads(&version, size, digests);
            }
            DownloadMsg::DockerDownloadProgress(version, progress) => {
                self.docker_download_progress(&version, progress);
            }
            DownloadMsg::DockerBlobFinished(version, digest) => {
                debug!("Finished download of {} digest {}", version, digest);
                self.docker_blob_finished(&version, &digest);
            }
            DownloadMsg::DockerBlobFailed(version, digest) => {
                self.download_docker_digest(&version, digest);
            }
            DownloadMsg::DockerExtractionFinished(version) => {
                self.docker_extraction_finished(&version);
            }
            DownloadMsg::IOError(e) => {
                if let Some(w) = self_.window.get() {
                    w.add_notification(
                        "iodownloaderror",
                        &format!("Unable to download file: {}", e),
                        gtk4::MessageType::Error,
                    );
                }
            }
            DownloadMsg::PauseChunk(url, path, guid) => {
                self.pause_asset_chunk(url, path, guid);
            }
            DownloadMsg::CancelChunk(_url, path, guid) => {
                self.remove_chunk(path, guid);
            }
            DownloadMsg::DockerCanceled(version, digest) => {
                self.cancel_docker_digest(&version, digest);
            }
            DownloadMsg::DockerPaused(version, digest) => {
                self.pause_docker_digest(version, digest);
            }
        }
    }

    fn get_item(&self, id: &str) -> Option<EpicDownloadItem> {
        let self_ = self.imp();
        let mut items = self_.download_items.borrow_mut();
        items.get_mut(id).map(|i| i.clone())
    }

    fn finish(&self, item: &download_item::EpicDownloadItem) {
        let self_: &imp::EpicDownloadManager = self.imp();
        if let Some(mut child) = self_.downloads.first_child() {
            loop {
                let row = child.clone().downcast::<gtk4::ListBoxRow>().unwrap();
                if let Some(i) = row.first_child() {
                    if i.eq(item) {
                        self_.downloads.remove(&row);
                        break;
                    }
                }
                if let Some(next_child) = row.next_sibling() {
                    child = next_child;
                } else {
                    break;
                }
            }
        }
        self.set_property("has-items", self_.downloads.first_child().is_some());
        self.emit_by_name::<()>("tick", &[]);
    }

    fn unreal_vault_dir(&self, asset: &str) -> Option<String> {
        let self_ = self.imp();
        if let Some(i) = self.get_item(asset) {
            if let Some(target) = i.target() {
                return Some(target);
            }
        };
        self_
            .settings
            .strv("unreal-vault-directories")
            .get(0)
            .map(std::string::ToString::to_string)
    }

    fn finalize_file_download(&self, file: &str, file_details: asset::DownloadedFile) {
        let self_ = self.imp();
        info!("File finished: {}", file);
        self_.downloaded_files.borrow_mut().remove(file);
        let vaults = self_.settings.strv("unreal-vault-directories");
        let temp_dir = std::path::PathBuf::from(match vaults.first() {
            None => self_
                .settings
                .string("temporary-download-directory")
                .to_string(),
            Some(v) => v.to_string(),
        });
        for chunk in file_details.finished_chunks {
            if let Some(ch) = self_.downloaded_chunks.borrow_mut().get_mut(&chunk.guid) {
                ch.retain(|x| !x.eq(file));
                if ch.is_empty() {
                    let mut p = temp_dir.clone();
                    p.push(&file_details.release);
                    p.push("temp");

                    p.push(format!("{}.chunk", chunk.guid));
                    debug!("Removing chunk {}", p.as_path().to_str().unwrap());
                    if let Err(e) = std::fs::remove_file(p.clone()) {
                        error!("Unable to remove chunk file: {}", e);
                    };
                    if let Err(e) = std::fs::remove_dir(p.parent().unwrap()) {
                        debug!("Unable to remove the temp directory(yet): {}", e);
                    };
                    if let Err(e) = std::fs::remove_dir(p.parent().unwrap().parent().unwrap()) {
                        debug!("Unable to remove the temp directory(yet): {}", e);
                    };
                }
            }
        }
        self_
            .sender
            .send(DownloadMsg::FileExtracted(file_details.asset))
            .unwrap();
    }

    pub fn progress(&self) -> f32 {
        let self_ = self.imp();
        let items = self_.download_items.borrow().values().len();
        let mut progress = 0.0_f32;
        for item in self_.download_items.borrow().values() {
            progress += item.progress();
        }
        if items > 0 {
            progress / items as f32
        } else {
            0.0_f32
        }
    }

    pub fn download_thumbnail(
        &self,
        image: egs_api::api::types::asset_info::KeyImage,
        asset: egs_api::api::types::asset_info::AssetInfo,
        sender: gtk4::glib::Sender<crate::ui::messages::Msg>,
    ) {
        let self_ = self.imp();
        let cache_dir = self_.settings.string("cache-directory").to_string();
        let mut cache_path = PathBuf::from(cache_dir);
        cache_path.push("images");
        let name = Path::new(image.url.path())
            .extension()
            .and_then(OsStr::to_str);
        cache_path.push(format!("{}.{}", image.md5, name.unwrap_or(".png")));
        self_.thumbnail_pool.execute(move || {
            if let Ok(w) = crate::RUNNING.read() {
                if !*w {
                    return;
                }
            }
            if let Ok(response) = reqwest::blocking::get(image.url.clone()) {
                if let Ok(b) = response.bytes() {
                    std::fs::create_dir_all(&cache_path.parent().unwrap()).unwrap();
                    //TODO: Report downloaded size
                    match File::create(cache_path.as_path()) {
                        Ok(mut thumbnail) => {
                            thumbnail.write_all(&b).unwrap();
                        }
                        Err(e) => {
                            error!("{:?}", e);
                        }
                    }
                    sender
                        .send(crate::ui::messages::Msg::ProcessAssetInfo(asset))
                        .unwrap();
                }
            };
        });
    }

    pub(crate) fn add_thread_sender(
        &self,
        key: String,
        sender: std::sync::mpsc::Sender<ThreadMessages>,
    ) {
        let self_ = self.imp();
        self_
            .thread_senders
            .borrow_mut()
            .entry(key)
            .or_insert(vec![])
            .push(sender);
    }

    pub(crate) fn send_to_thread_sender(&self, key: &str, msg: &ThreadMessages) {
        let self_ = self.imp();
        if let Some(senders) = self_.thread_senders.borrow_mut().remove(key) {
            for sender in senders {
                if sender.send(msg.clone()).is_err() {
                    warn!("Unable to send message {:?} to {}", msg, key);
                };
            }
        }
    }

    pub fn download_image(
        &self,
        image: egs_api::api::types::asset_info::KeyImage,
        asset: String,
        sender: gtk4::glib::Sender<crate::ui::widgets::logged_in::library::image_stack::ImageMsg>,
    ) {
        let self_ = self.imp();
        let cache_dir = self_.settings.string("cache-directory").to_string();
        let mut cache_path = PathBuf::from(cache_dir);
        cache_path.push("images");
        let name = Path::new(image.url.path())
            .extension()
            .and_then(OsStr::to_str);
        cache_path.push(format!("{}.{}", image.md5, name.unwrap_or(".png")));
        let img = image.clone();
        self_.image_pool.execute(move || {
            if let Ok(w) = crate::RUNNING.read() {
                if !*w {
                    return;
                }
            }
            debug!("Downloading image");
            if let Ok(response) = reqwest::blocking::get(image.url.clone()) {
                if let Ok(b) = response.bytes() {
                    std::fs::create_dir_all(&cache_path.parent().unwrap()).unwrap();
                    //TODO: Report downloaded size
                    match File::create(cache_path.as_path()) {
                        Ok(mut thumbnail) => {
                            thumbnail.write_all(&b).unwrap();
                        }
                        Err(e) => {
                            error!("{:?}", e);
                        }
                    }
                    sender
                        .send(
                            crate::ui::widgets::logged_in::library::image_stack::ImageMsg::LoadImage(
                                asset, img,
                            ),
                        )
                        .unwrap();
                }
            };
        });
    }
}
