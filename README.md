# vrm-rs

VRM/VRMA の読み込み、アニメーション、MToon レンダリングを扱う Rust ワークスペースです。

## WebAssembly サンプル

wgpu ベースのブラウザサンプルを GitHub Pages で公開しています。

**[WASM サンプルを開く](https://sanzentyo.github.io/vrm-rs/wasm-example/)**

- 左ドラッグ: オービット
- 右または中ドラッグ: パン
- ホイール: ズーム
- ダブルクリック: カメラをリセット

ローカルでのビルド方法と、サンプルが参照する外部アセットについては
[`examples/wasm/wgpu-web/README.md`](examples/wasm/wgpu-web/README.md) を参照してください。

## License

ソースコードは、利用者が選択する [MIT License](LICENSE-MIT) または
[Apache License 2.0](LICENSE-APACHE) のいずれかで提供します。

VRM/VRMA のサンプルバイナリはリポジトリや GitHub Pages に同梱していません。
WebAssembly サンプルの初期 URL は各上流リポジトリを直接参照します。外部アセットには
それぞれの作者・配布元・VRM メタデータの利用条件が適用されます。
