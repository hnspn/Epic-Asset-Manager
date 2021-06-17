mod asset;
pub mod category;

use crate::tools::asset_info::Search;
use crate::ui::widgets::logged_in::asset::EpicAsset;
use glib::clone;
use gtk::{self, gdk_pixbuf, prelude::*};
use gtk::{gio, glib, subclass::prelude::*, CompositeTemplate};
use gtk_macros::action;
use log::{debug, error};
use std::ffi::OsStr;
use std::fs;
use std::fs::File;
use std::io::Read;
use std::ops::Not;
use std::path::{Path, PathBuf};

pub(crate) mod imp {
    use super::*;
    use crate::window::EpicAssetManagerWindow;
    use gtk::gio;
    use gtk::gio::ListStore;
    use gtk::glib::{Object, ParamSpec};
    use once_cell::sync::OnceCell;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::sync::Arc;
    use threadpool::ThreadPool;

    #[derive(Debug, CompositeTemplate)]
    #[template(resource = "/io/github/achetagames/epic_asset_manager/logged_in.ui")]
    pub struct EpicLoggedInBox {
        #[template_child]
        pub home_category:
            TemplateChild<crate::ui::widgets::logged_in::category::EpicSidebarCategory>,
        #[template_child]
        pub assets_category:
            TemplateChild<crate::ui::widgets::logged_in::category::EpicSidebarCategory>,
        #[template_child]
        pub plugins_category:
            TemplateChild<crate::ui::widgets::logged_in::category::EpicSidebarCategory>,
        #[template_child]
        pub games_category:
            TemplateChild<crate::ui::widgets::logged_in::category::EpicSidebarCategory>,
        #[template_child]
        pub expand_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub expand_image: TemplateChild<gtk::Image>,
        #[template_child]
        pub expand_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub asset_grid: TemplateChild<gtk::GridView>,
        pub sidebar_expanded: RefCell<bool>,
        pub actions: gio::SimpleActionGroup,
        pub window: OnceCell<EpicAssetManagerWindow>,
        pub grid_model: ListStore,
        pub loaded_assets: RefCell<HashMap<String, egs_api::api::types::asset_info::AssetInfo>>,
        pub asset_load_pool: ThreadPool,
        pub image_load_pool: ThreadPool,
        pub assets_pending: Arc<std::sync::RwLock<Vec<Object>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for EpicLoggedInBox {
        const NAME: &'static str = "EpicLoggedInBox";
        type Type = super::EpicLoggedInBox;
        type ParentType = gtk::Box;

        fn new() -> Self {
            Self {
                home_category: TemplateChild::default(),
                assets_category: TemplateChild::default(),
                plugins_category: TemplateChild::default(),
                games_category: TemplateChild::default(),
                expand_button: TemplateChild::default(),
                expand_image: TemplateChild::default(),
                expand_label: TemplateChild::default(),
                asset_grid: TemplateChild::default(),
                sidebar_expanded: RefCell::new(false),
                actions: gio::SimpleActionGroup::new(),
                window: OnceCell::new(),
                grid_model: gio::ListStore::new(crate::models::row_data::RowData::static_type()),
                loaded_assets: RefCell::new(HashMap::new()),
                asset_load_pool: ThreadPool::with_name("Asset Load Pool".to_string(), 5),
                image_load_pool: ThreadPool::with_name("Image Load Pool".to_string(), 5),
                assets_pending: Arc::new(std::sync::RwLock::new(vec![])),
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

    impl ObjectImpl for EpicLoggedInBox {
        fn properties() -> &'static [ParamSpec] {
            use once_cell::sync::Lazy;
            static PROPERTIES: Lazy<Vec<ParamSpec>> = Lazy::new(|| {
                vec![ParamSpec::new_boolean(
                    "sidebar-expanded",
                    "sidebar expanded",
                    "Is Sidebar expanded",
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
                "sidebar-expanded" => {
                    let sidebar_expanded = value.get().unwrap();
                    self.sidebar_expanded.replace(sidebar_expanded);
                }
                _ => unimplemented!(),
            }
        }

        fn property(&self, _obj: &Self::Type, _id: usize, pspec: &ParamSpec) -> glib::Value {
            match pspec.name() {
                "sidebar-expanded" => self.sidebar_expanded.borrow().to_value(),
                _ => unimplemented!(),
            }
        }

        fn constructed(&self, obj: &Self::Type) {
            self.parent_constructed(obj);
            obj.bind_properties();
            obj.setup_actions();
            self.plugins_category
                .add_category("Engine".to_string(), "plugins/engine".to_string());
            self.assets_category
                .add_category("2d".to_string(), "assets/2d".to_string());
            self.assets_category
                .add_category("animations".to_string(), "assets/animations".to_string());
            self.assets_category
                .add_category("archvis".to_string(), "assets/archvis".to_string());
            self.assets_category
                .add_category("blueprints".to_string(), "assets/blueprints".to_string());
            self.assets_category
                .add_category("characters".to_string(), "assets/characters".to_string());
            self.assets_category.add_category(
                "communitysamples".to_string(),
                "assets/communitysamples".to_string(),
            );
            self.assets_category.add_category(
                "environments".to_string(),
                "assets/environments".to_string(),
            );
            self.assets_category
                .add_category("fx".to_string(), "assets/fx".to_string());
            self.assets_category
                .add_category("materials".to_string(), "assets/materials".to_string());
            self.assets_category
                .add_category("megascans".to_string(), "assets/megascans".to_string());
            self.assets_category
                .add_category("music".to_string(), "assets/music".to_string());
            self.assets_category
                .add_category("props".to_string(), "assets/props".to_string());
            self.assets_category.add_category(
                "showcasedemos".to_string(),
                "assets/showcasedemos".to_string(),
            );
            self.assets_category
                .add_category("soundfx".to_string(), "assets/soundfx".to_string());
            self.assets_category
                .add_category("textures".to_string(), "assets/textures".to_string());
            self.assets_category
                .add_category("weapons".to_string(), "assets/weapons".to_string());
            self.home_category
                .add_category("all".to_string(), "".to_string());
            self.home_category
                .add_category("favorites".to_string(), "favorites".to_string());
        }
    }

    impl WidgetImpl for EpicLoggedInBox {}
    impl BoxImpl for EpicLoggedInBox {}
}

glib::wrapper! {
    pub struct EpicLoggedInBox(ObjectSubclass<imp::EpicLoggedInBox>)
        @extends gtk::Widget, gtk::Box;
}

impl EpicLoggedInBox {
    pub fn new() -> Self {
        let stack: Self = glib::Object::new(&[]).expect("Failed to create EpicLoggedInBox");

        stack
    }

    pub fn set_window(&self, window: &crate::window::EpicAssetManagerWindow) {
        let self_: &imp::EpicLoggedInBox = imp::EpicLoggedInBox::from_instance(self);
        // Do not run this twice
        if let Some(_) = self_.window.get() {
            return;
        }

        self_.window.set(window.clone()).unwrap();
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(move |_factory, item| {
            let row = EpicAsset::new();
            item.set_child(Some(&row));
        });

        factory.connect_bind(move |_factory, list_item| {
            let data = list_item
                .item()
                .unwrap()
                .downcast::<crate::models::row_data::RowData>()
                .unwrap();

            let child = list_item.child().unwrap().downcast::<EpicAsset>().unwrap();
            child.set_property("label", &data.name()).unwrap();
            child.set_property("thumbnail", &data.image()).unwrap();
        });

        let sorter = gtk::CustomSorter::new(move |obj1, obj2| {
            let info1 = obj1
                .downcast_ref::<crate::models::row_data::RowData>()
                .unwrap();
            let info2 = obj2
                .downcast_ref::<crate::models::row_data::RowData>()
                .unwrap();

            info1
                .name()
                .to_lowercase()
                .cmp(&info2.name().to_lowercase())
                .into()
        });

        let sorted_model = gtk::SortListModel::new(Some(&self_.grid_model), Some(&sorter));
        let selection_model = gtk::SingleSelection::new(Some(&sorted_model));
        self_.asset_grid.set_model(Some(&selection_model));
        self_.asset_grid.set_factory(Some(&factory));
        self.fetch_assets();
        glib::idle_add_local(clone!(@weak self as obj => @default-panic, move || {
            obj.flush_assets();

            glib::Continue(true)
        }));
    }

    pub fn flush_assets(&self) {
        let start = std::time::Instant::now();
        let self_: &imp::EpicLoggedInBox = imp::EpicLoggedInBox::from_instance(self);
        if let Ok(mut vec) = self_.assets_pending.write() {
            if vec.is_empty() {
                return;
            }
            self_.grid_model.splice(0, 0, vec.as_slice());
            vec.clear();
        }
        debug!("Finished flushing {:?}", start.elapsed());
    }

    pub fn bind_properties(&self) {
        let self_: &imp::EpicLoggedInBox = imp::EpicLoggedInBox::from_instance(self);
        self.bind_property("sidebar-expanded", &*self_.home_category, "expanded")
            .flags(glib::BindingFlags::DEFAULT | glib::BindingFlags::SYNC_CREATE)
            .build();
        self.bind_property("sidebar-expanded", &*self_.assets_category, "expanded")
            .flags(glib::BindingFlags::DEFAULT | glib::BindingFlags::SYNC_CREATE)
            .build();
        self.bind_property("sidebar-expanded", &*self_.plugins_category, "expanded")
            .flags(glib::BindingFlags::DEFAULT | glib::BindingFlags::SYNC_CREATE)
            .build();
        self.bind_property("sidebar-expanded", &*self_.games_category, "expanded")
            .flags(glib::BindingFlags::DEFAULT | glib::BindingFlags::SYNC_CREATE)
            .build();
    }

    pub fn setup_actions(&self) {
        let self_: &imp::EpicLoggedInBox = imp::EpicLoggedInBox::from_instance(self);

        action!(
            self_.actions,
            "expand",
            clone!(@weak self as win => move |_, _| {
                    if let Ok(v) = win.property("sidebar-expanded") {
                    let self_: &imp::EpicLoggedInBox = imp::EpicLoggedInBox::from_instance(&win);
                    let new_value = !v.get::<bool>().unwrap();
                    if new_value {
                        self_.expand_image.set_icon_name(Some("go-previous-symbolic"));
                        self_.expand_button.set_tooltip_text(Some("Collapse Sidebar"));
                        self_.expand_label.set_label("Collapse");
                    } else {
                        self_.expand_image.set_icon_name(Some("go-next-symbolic"));
                        self_.expand_button.set_tooltip_text(Some("Expand Sidebar"));
                        self_.expand_label.set_label("");
                    };
                    win.set_property("sidebar-expanded", &new_value).unwrap();
                }
            })
        );
        self.insert_action_group("loggedin", Some(&self_.actions));
    }

    pub fn add_asset(&self, asset: egs_api::api::types::asset_info::AssetInfo, image: &[u8]) {
        let self_: &imp::EpicLoggedInBox = imp::EpicLoggedInBox::from_instance(self);
        let start = std::time::Instant::now();
        let mut assets = self_.loaded_assets.borrow_mut();
        if match assets.get_mut(&asset.id) {
            None => {
                assets.insert(asset.id.clone(), asset.clone());
                true
            }
            Some(a) => {
                if asset.eq(a) {
                    debug!("Duplicate asset: {}", asset.id);
                    false
                } else {
                    assets.insert(asset.id.clone(), asset.clone());
                    true
                }
            }
        } {
            if let Some(name) = asset.title {
                debug!("{} - Starting image load took {:?}", name, start.elapsed());
                let data =
                    crate::models::row_data::RowData::new(Some(asset.id), name.clone(), image);
                debug!("{} - Finished image load took {:?}", name, start.elapsed());
                if let Ok(mut vec) = self_.assets_pending.write() {
                    vec.push(data.upcast());
                }
                debug!("{} - Finished appending {:?}", name, start.elapsed());
            } else {
                error!("Asset {} does not have name", asset.id);
            }
        }
    }

    pub fn load_thumbnail(&self, asset: egs_api::api::types::asset_info::AssetInfo) {
        let self_: &imp::EpicLoggedInBox = imp::EpicLoggedInBox::from_instance(self);
        if let Some(window) = self.main_window() {
            let win_ = window.data();
            let sender = win_.model.sender.clone();
            match asset.thumbnail() {
                None => {
                    sender
                        .send(crate::ui::messages::Msg::ProcessAssetThumbnail(
                            asset,
                            vec![],
                        ))
                        .unwrap();
                }
                Some(t) => {
                    let cache_dir = win_
                        .model
                        .settings
                        .string("cache-directory")
                        .to_string()
                        .clone();
                    let mut cache_path = PathBuf::from(cache_dir);
                    cache_path.push("images");
                    let name = Path::new(t.url.path()).extension().and_then(OsStr::to_str);
                    cache_path.push(format!("{}.{}", t.md5, name.unwrap_or(&".png")));
                    self_.image_load_pool.execute(move || {
                        match File::open(cache_path.as_path()) {
                            Ok(mut f) => {
                                let metadata = fs::metadata(&cache_path.as_path())
                                    .expect("unable to read metadata");
                                let mut buffer = vec![0; metadata.len() as usize];
                                f.read(&mut buffer).expect("buffer overflow");
                                let pixbuf_loader = gdk_pixbuf::PixbufLoader::new();
                                pixbuf_loader.write(&buffer).unwrap();
                                pixbuf_loader.close().ok();
                                match pixbuf_loader.pixbuf() {
                                    None => {}
                                    Some(pb) => {
                                        let width = pb.width();
                                        let height = pb.height();

                                        let width_percent = 128.0 / width as f64;
                                        let height_percent = 128.0 / height as f64;
                                        let percent = if height_percent < width_percent {
                                            height_percent
                                        } else {
                                            width_percent
                                        };
                                        let desired =
                                            (width as f64 * percent, height as f64 * percent);
                                        sender
                                            .send(crate::ui::messages::Msg::ProcessAssetThumbnail(
                                                asset,
                                                pb.scale_simple(
                                                    desired.0.round() as i32,
                                                    desired.1.round() as i32,
                                                    gdk_pixbuf::InterpType::Bilinear,
                                                )
                                                .unwrap()
                                                .save_to_bufferv("png", &[])
                                                .unwrap(),
                                            ))
                                            .unwrap()
                                    }
                                };
                            }
                            Err(_) => {
                                println!("Need to load image");
                            }
                        };
                    })
                }
            }
        }
    }

    fn main_window(&self) -> Option<&crate::window::EpicAssetManagerWindow> {
        let self_: &imp::EpicLoggedInBox = imp::EpicLoggedInBox::from_instance(self);
        match self_.window.get() {
            Some(window) => Some(&(*window)),
            None => None,
        }
    }

    pub fn fetch_assets(&self) {
        let self_: &imp::EpicLoggedInBox = imp::EpicLoggedInBox::from_instance(self);
        if let Some(window) = self.main_window() {
            let win_ = window.data();
            let sender = win_.model.sender.clone();
            let cache_dir = win_
                .model
                .settings
                .string("cache-directory")
                .to_string()
                .clone();
            self_.asset_load_pool.execute(move || {
                // Load assets from cache
                let cache_path = PathBuf::from(cache_dir);
                if cache_path.is_dir() {
                    for entry in std::fs::read_dir(cache_path).unwrap() {
                        if let Ok(w) = crate::RUNNING.read() {
                            if w.not() {
                                break;
                            }
                        }
                        let mut asset_file = entry.unwrap().path();
                        asset_file.push("asset_info.json");
                        if asset_file.exists() {
                            if let Ok(mut f) = std::fs::File::open(asset_file.as_path()) {
                                let mut buffer = String::new();
                                f.read_to_string(&mut buffer).unwrap();
                                if let Ok(asset) = serde_json::from_str::<
                                    egs_api::api::types::asset_info::AssetInfo,
                                >(&buffer)
                                {
                                    sender
                                        .send(crate::ui::messages::Msg::ProcessAssetInfo(asset))
                                        .unwrap();
                                }
                            };
                        }
                    }
                }
                // TODO: Update from the API
            })
        }
    }
}
