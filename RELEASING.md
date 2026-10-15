# Android 配布版の作成

Android の APK 更新には同じアプリ ID、同じ署名鍵、増加する `versionCode` が必要です。鍵と `keystore.properties` は Git に追加しません。鍵を失うと、現在のアプリ ID のまま既存インストールへ更新を配布できなくなります。

1. `package.json` と `src-tauri/tauri.conf.json` の版を揃えて更新する。Tauri はこの版から Android の `versionCode` を生成する。
2. `src-tauri/gen/android/keystore.properties` に `storeFile`、`storePassword`、`keyAlias`、`keyPassword` を設定する。署名鍵はリポジトリ外に保管する。
3. `npm ci` と `npm run build` を実行する。
4. PowerShell で `$env:RUSTFLAGS = "--remap-path-prefix=$env:USERPROFILE=/build"` を設定してから、同じシェルで `npx tauri android build --apk --target aarch64 --ci` を実行する。Rust の成果物へビルド端末のユーザー名やホームディレクトリを埋め込まないための設定です。
5. 出力 APK の署名、アプリ ID、版を Android SDK の `apksigner` と `aapt` で確認する。APK 内のファイルを展開して、ローカルのユーザープロファイルのパスが含まれていないことも確認する。
6. 変更をコミットして `v<版>` タグを付け、GitHub Releases に ARM64 APK を添付する。APK の SHA-256 と、実機未検証などの制限をリリースノートに書く。

公開済みの版を作り直す場合も署名鍵を変更しない。開発用デバッグ APK は配布版へ上書きできない。署名鍵とパスワードはリポジトリとは別の安全な場所にバックアップする。
