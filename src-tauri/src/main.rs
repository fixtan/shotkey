// リリースビルドでは、黒い窓 (コンソール) を出さない
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    shotkey_lib::run()
}
