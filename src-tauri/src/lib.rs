mod avd;
mod ide;
mod jdk;
mod sdk;
mod util;

use std::sync::atomic::AtomicBool;
use std::sync::Mutex;
use util::SdkTask;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(SdkTask {
            child: Mutex::new(None),
            cancelled: AtomicBool::new(false),
        })
        .invoke_handler(tauri::generate_handler![
            sdk::sdk_status,
            sdk::check_hardware_accel,
            sdk::check_network,
            jdk::check_java,
            sdk::open_windows_features,
            sdk::sdk_path,
            ide::ide_integration_status,
            ide::enable_ide_integration,
            ide::disable_ide_integration,
            avd::list_device_profiles,
            sdk::preferred_abi,
            avd::rotate_avd,
            ide::detect_connected_ides,
            sdk::install_sdk,
            sdk::cancel_sdk_task,
            sdk::list_available_images,
            sdk::download_image,
            avd::list_avds,
            avd::list_running_avds,
            avd::create_avd,
            avd::delete_avd,
            avd::launch_avd,
            avd::stop_avd,
            avd::list_snapshots,
            avd::save_snapshot,
            avd::load_snapshot,
            avd::delete_snapshot,
            avd::install_apk,
            util::nuke_all,
            util::check_disk_space,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
