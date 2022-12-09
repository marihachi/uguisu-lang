言語仕様を考える

## 変数宣言
変数
```
let a = 123;
```

再代入不可な変数
```
const b = 123;
```

## 関数
### 関数の定義
関数は以下のように定義する。
推論されないため、関数のシグネチャは省略できない。
```
fn add(a: number, b: number): number {
  return a + b;
}
```

### 外部関数の宣言
外部関数としてコンパイラに指示するには、externalを指定する。
外部関数は関数の宣言のみ必要なため、ブロックではなくセミコロンにする。
ビルトイン関数は外部関数として宣言される。

```
external fn add(a: number, b: number): number;
```

## モジュール
1つのファイルを1つのモジュールとする。

### モジュールの公開
export文を使って、メンバを公開できる。

関数の公開
```
export fn add(a: number, b: number): number {
  return a + b;
}
```

### 他のモジュールへのアクセス
import文を使って、他のモジュールの公開メンバにアクセスできる。

```
import MathUtil;

fn main(): void {
  let num = MathUtil.add(1, 2);
}
```

### ネームスペース
モジュールがあるフォルダの階層がそのモジュールが公開されるパスになる。
これをネームスペースと呼ぶ。

モジュールのファイルパス(srcディレクトリからの相対パス)が`Hoge/Piyo/ExampleModule.**`の場合、
このモジュールのネームスペースは「Hoge::Piyo」となる。

他のモジュールからインポートするには以下のように書く。
```
import Hoge::Piyo::ExampleModule;
```