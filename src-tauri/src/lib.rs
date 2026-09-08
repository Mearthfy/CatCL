mod accounts;
mod downloads;
mod java_runtime;
mod launch;
mod loaders;
mod modpacks;
pub mod p2p;
mod resources;
mod settings;
mod skins;
mod worlds;

use downloads::DownloadState;
use launch::GameState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(DownloadState::default())
        .manage(GameState::default())
        .manage(accounts::AccountState::default())
        .manage(p2p::port_mapping::MappingState::default())
        .invoke_handler(tauri::generate_handler![
            settings::load_settings,
            settings::save_settings,
            settings::scan_instance_folder,
            settings::delete_instance,
            settings::detect_java,
            accounts::load_account,
            accounts::login_littleskin,
            accounts::start_microsoft_login,
            accounts::finish_microsoft_login,
            accounts::open_microsoft_login,
            accounts::logout_account,
            downloads::list_versions,
            downloads::install_version,
            downloads::cancel_install,
            downloads::installed_versions,
            worlds::list_worlds,
            worlds::rename_world,
            worlds::import_world,
            worlds::open_world_folder,
            launch::launch_game,
            loaders::install_loader,
            loaders::list_loader_versions,
            modpacks::inspect_modpack,
            modpacks::apply_modpack,
            resources::search_resources,
            resources::open_resource_site,
            resources::resource_versions,
            resources::download_resource,
            resources::install_resource,
            java_runtime::install_java,
            skins::import_skin,
            skins::load_skin,
            skins::remove_skin,
            p2p::minecraft::detect_minecraft_lan,
            p2p::network::inspect_p2p_network,
            p2p::nat::detect_p2p_nat,
            p2p::port_mapping::create_port_mapping,
            p2p::port_mapping::remove_port_mapping
        ])
        .run(tauri::generate_context!())
        .expect("无法启动 CatCL");
}
