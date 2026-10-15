# Android 開発環境

このリポジトリは Windows 版 piep の閲覧用 Android アプリです。デスクトップ版は別リポジトリで開発します。Android Studio、Android SDK、JDK 17、Rust の Android ターゲット、Node.js を用意します。Tauri 2 の [Android 前提条件](https://v2.tauri.app/start/prerequisites/) を参照してください。

## エミュレーターで起動

Android Studio の Device Manager で仮想端末を起動し、このリポジトリで次を実行します。

```powershell
npm ci
npx tauri android dev
```

Windows では Tauri のシンボリックリンク生成に開発者モードが必要です。JDK の版が合わない場合は `JAVA_HOME` を JDK 17 のインストール先に設定してください。

PC 版の本棚に接続するには、共有機能を含むデスクトップ版で「設定 → Android で読む」から共有を開始し、Android 版の「設定 → PC との接続」に登録文字列を貼り付けます。エミュレーターから PC のループバック接続口へつなぐ場合は、接続前に次の転送を設定できます。

```powershell
adb reverse tcp:38777 tcp:38777
```

転送はエミュレーターの再起動などで消えることがあります。`adb reverse --list` で確認できます。家庭内ネットワークから接続する場合は、PC 側の共有先に LAN アドレスを選びます。

## 確認

```powershell
npm run build
npx tauri android build --debug --apk
```

デバッグ APK は動作確認用です。公開配布には署名済みリリースビルドと配布手順が別途必要です。接続方式は [PROTOCOL.md](PROTOCOL.md)、デスクトップ版との表示対応は [DESKTOP-PARITY.md](DESKTOP-PARITY.md) を参照してください。
