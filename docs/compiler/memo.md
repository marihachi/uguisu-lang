## 構文解析
ソースコードを読み取り、ASTを組み立てる。

## 意味解析
関数宣言や変数宣言を見て、識別子の解決に必要な情報や型情報(スコープ情報)を収集する。
識別子を見て、スコープに存在する関数や変数への解決を行う。
AST全体で、型の矛盾がないかどうかを確認する。

### スコープ情報
関数や変数をシンボルとして登録する。
そのスコープに存在する全てのシンボルと、各シンボルの型情報にアクセスできる。
スコープの構造はレイヤー状になっている。1つのレイヤーには複数のシンボルを登録できる。
現在のレイヤーから親のレイヤーには遡ってアクセスできる。子のレイヤーにはアクセスできない。

## コード生成
ASTをトラバースしてコードを生成する。
