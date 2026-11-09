# Android の名前空間別保管

共通の `piep-storage` を固定 Git revision で使う、UI と DB に依存しないネイティブ層。
デスクトップのローカル checkout や端末向けの別デコーダーを必要としない。

`StorageNamespace` は、登録接続・棚・認証された利用者の三つの UUID を保持する。
利用者は端末 ID と区別し、認証済みの API 応答から得る。URL・表示名から推測しない。
ネイティブ側が得たアプリ専用 root の下だけに保存し、各階層でリンク・通常ファイルを
拒否する。他の名前空間のハッシュを知っていても再利用対象にしない。

`receive_chunk` の成功は一部分、`verify_file` の成功は一ファイルのバイト列の検証。
`VerifiedFile` は検証したストアを参照し、別の利用者へ検証結果を持ち替えられない。
読み出しでも実体を検査する。圧縮済みの受信内容は形式・容量・展開後・SHA-256 を
共通処理で検査し、中断した `.part` だけを除き、正常な既存実体は残す。

この層は保存完了や認可を記録しない。端末 DB と repository は後で、要求した作品・版の
所属、connection/sync epoch、全 file と読書 bundle の完成を確認し、現在参照を
トランザクションで切り替える。ネットワーク取得、認証鍵、Kotlin の実行制御も別の責務。
並行する削除・GC は未接続であり、この参照を永続 pin の代わりには使わない。

現在のアプリはこの crate を native dependency に持つが、旧保存物の移行や API v2、
端末 DB・読書画面からはまだ呼び出さない。本番の保存先や既存の読書を変更しない。

```sh
cargo test --manifest-path crates/piep-android-storage/Cargo.toml --locked
cargo test --manifest-path crates/piep-android-storage/Cargo.toml --locked --target aarch64-linux-android --no-run
```
