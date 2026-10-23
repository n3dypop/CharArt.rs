# CharArt.rs

画像・GIF・動画を文字アートへ変換する Rust CLI / ライブラリです。
ASCII、Braille、半ブロック、モノクロ・カラー、自動調整に対応します。
クレート名・コマンド名は `charart` です。

## ビルドと依存関係

Rust 1.89 以降が必要です（動作確認: 1.99）。CPU 版に CUDA は不要です。

```sh
cargo install --path . --locked
charart --help
```

初回は依存取得にネットワークが必要です。
画像・GIF は追加ソフトなしで読み込みます。動画には PATH 上の `ffmpeg` と `ffprobe` が必要です。
macOS: `brew install ffmpeg`、Ubuntu: `sudo apt install ffmpeg`。
Windows は [FFmpeg の配布案内](https://ffmpeg.org/download.html)を参照してください。FFmpeg は同梱しません。

## 使い方

```sh
charart photo.png --auto                         # 画像に合わせて自動調整
charart photo.png --width 80 --mode braille       # 幅・表現を指定
charart photo.png --charset ' .oO@' --invert      # 文字セット・明暗を指定
charart photo.png --color -o art.html            # カラーで保存
charart photo.png --color -o header.lua          # alpha-nvim 用
charart animation.gif --play --loop --speed 1.5  # ターミナル再生
charart clip.mp4 --color --play
charart photo.png --analyze -o settings.json     # 自動設定を保存
charart photo.png --profile settings.json -o art.txt
```

入力は PNG/JPEG/BMP/PNM/WebP、GIF、FFmpeg 対応動画です。
出力は Text/ANSI/HTML/Lua/JSONL。保存先の拡張子または `--format` で選びます。
HTML/Lua に GIF・動画を保存する場合は `--frames 1` を指定します。
幅の未指定時は端末幅に合わせます。高さ・倍率・セル縦横比、しきい値、背景色、
色量子化、ディザリング、明度・コントラスト・ガンマも調整できます。詳細は `--help` を参照してください。

## ライブラリ

```rust
use charart::{Backend, Converter, Options, output::{self, Format}};
let image = image::open("photo.png")?.into_rgba8();
let mut converter = Converter::new(Options { backend: Backend::Cpu, ..Default::default() })?;
let art = converter.convert(&image)?;
let text = output::render(&art, Format::Text, false);
```

ライブラリは標準出力への書込やプロセス終了を行いません。

## CUDA（任意）

[NVIDIA/cuda-rust](https://github.com/NVIDIA/cuda-rust) の cuTile で透過合成・輝度計算・色量子化を処理します。
Linux、対応 NVIDIA GPU（sm_80 以降）、ドライバー、libclang、`tileiras` を含む CUDA Toolkit が必要です。
Toolkit 13.3 以降を推奨します（sm_107 は 13.4 以降）。

```sh
export CUDA_TOOLKIT_PATH=/usr/local/cuda-13.3
cargo build --release --locked --features cuda
./target/release/charart photo.png --backend cuda --color
```

`--backend auto|cpu|cuda` で選択します。auto は利用失敗の理由を通知して CPU に切り替え、
cuda の明示指定ではエラーにします。GPU 実機での動作・性能は未検証です。

## 配布・制約

`v<version>` タグで Linux x86_64/ARM64、macOS Intel/Apple Silicon、Windows x86_64 の
CPU バイナリをビルドします。既存タグを指定した手動実行も可能です。
バイナリ・チェックサム・依存ライセンスは Actions の Artifacts に保存します（30日）。

最近傍リサイズ、音声なし。APNG・アニメーション WebP は最初の画像のみです。
出力は 4096 セル/軸・総数 1,048,576 セル、入力は 33,554,432 画素までです。

[MIT ライセンス](LICENSE)。依存先の許諾文は配布バイナリにも同梱します。
