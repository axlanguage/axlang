# Ax AI-Min Syntax Reference

Use this when authoring or repairing `.ax` files. Ax source is AI-min source; do not write an expanded variant and do not run a minification step.

## Top-Level Items

```txt
+pack.name                         // external pack import
{...}                              // bare main, returns i32
@name(a:#,b:$):ret{...}            // function
@name(a:#):ret!effect.one,e.two{...}
@@name(a:#):ret{...}               // async function
%Record{field:#,name:$}            // record type
%%Enum{A,B}                        // enum
%!Err{Bad,Missing}                 // error enum
?"test name"{...}                  // pure compiler-side test block
&3000{...}                         // HTTP server
```

TLS server markers:

```txt
&!3443{G/ping>"pong"}              // HTTP TLS flag
&&!3444{"ping">"pong"}             // TCP TLS flag
```

## Types

`#` is `i32` and `$` is `str` in type positions. Other primitive types are `i8`, `i16`, `i32`, `i64`, `u8`, `u16`, `u32`, `u64`, `f32`, `f64`, `bool`, `str`, and `void`.

```txt
@a(x:#):#          // i32 -> i32
@b(s:$):$          // str -> str
@c(xs:[#]):#       // array of i32
@d(x:#?):#         // optional i32
@e(r:Result<#,$>):#
```

Use explicit widths for public boundaries, native interop, record fields, and arithmetic where coercion would be ambiguous.

## Blocks And Statements

Blocks are `{...}`. Statements are parsed by leading token and can be adjacent when the parser can separate them; add spaces where adjacency changes meaning.

```txt
{$x=1 $y=2 ^x+y}
```

Local bindings:

```txt
$x=expr             // named local
$x:i64=expr         // typed local
$=expr              // implicit generated min name
$expr               // implicit generated min name
```

Control and return:

```txt
^expr               // return expr
^                   // return void
?cond{...}|{...}    // if / else
?cond{...}          // if
~cond{...}          // while
~{...}              // infinite loop
:expr               // assert truthy
```

Assignment accepts identifiers, fields, and supported l-values:

```txt
x=x+1
user.id=2
```

Semicolon std statements:

```txt
;value              // io.println(value)
;path,value         // fs.write_text(path,value)
;;path,json         // fs.write_json_atomic(path,json)
;+path              // fs.mkdir(path)
```

String pools reduce repeated text:

```txt
@{#(a"alpha",b"beta");a;b^0}
@{#("alpha","beta");a;b^0}         // implicit names a, b
```

## Expressions

Primary expressions include integers, floats, strings, identifiers, calls, record init, and JSON object shorthand.

```txt
add(1,2)
user.name
User{id:1,name:"ax"}
{"ok":!1}
{"name"="ax"}
```

Operators, lowest to highest precedence:

```txt
|                    boolean or
&                    boolean and
: !:                 equality / inequality
< <= > >=            comparisons
+ -                  add / subtract
* / %                multiply / divide / modulo
```

Unary and literal forms:

```txt
!1                   // true
!0                   // false
!flag                // not
-n                   // negate
@(future)            // await
```

Adjacent postfixes:

```txt
s!                   // str.len(s)
s~needle             // str.contains(s,needle)
path@                // fs.is_file(path)
j`"user.name"        // json.query(j,"user.name")
j'"ok"               // json.query_bool(j,"ok")
j#"count"            // json.query_int(j,"count")
j\"items"            // json.query_len(j,"items")
j["items",0]         // json.at(j,"items",0)
```

Spacing matters for adjacent postfixes:

```txt
$a=b!                // str.len(b)
! b                  // unary not b
$a=b~c               // str.contains(b,c)
~ b{...}             // while b
```

## Tests

Pure tests use `?"name"{...}` and assertions.

```txt
@fib(n:#):#{$i=0$a=0$b=1 ~i<n{$next=a+b a=b b=next i=i+1}^a}
?"fib"{:fib(7):13}
```

`:expr` asserts truthy. `:actual:expected` asserts equality.

## HTTP Server DSL

HTTP item:

```txt
&3000{G/ping>"pong" P/echo>~ G/state>#{ok:!1,service:"ax"}}
```

Methods are `G` GET, `P` POST, `U` PUT, `A` PATCH, and `D` DELETE. Responses:

```txt
G/ping>"pong"                       // text response
G/state>#{ok:!1,count:1}            // JSON object response
P/echo>~                            // stream request body back
G/assets/*>"asset"                  // wildcard path
```

## TCP Server DSL

TCP item:

```txt
&&3000{"PING\n">"PONG\n"*>"ERR\n"}
```

Routes are exact string matches or `*` wildcard, and responses are strings. For custom TCP servers, use `Qa/Qb/Qc` plus `TcpServer` and `TcpConn` methods.

```txt
@handler(state:Map,line:$):${^"+PONG\r\n"}
{$srv=Qa("127.0.0.1",6379)$m=La(1024)$workers=2^Qc(srv,m,"handler",workers)}
```

Low-level connection methods include `accept`, `read_text`, `write_text`, `request_text`, and `close`.

## Async

Declare async functions with `@@`. Calling an async function returns a future. Await with prefix `@`; detach with `Nb(future)`; cancel with `Na(future)`.

```txt
@@work(x:#):#{^x+1}
{^@(work(41))}
```

Futures and heap pointers are move-only. Do not await, detach, cancel, free, or field-move the same value twice.

## Compact Std Aliases

Prefer compact aliases when generating AI-min source. Aliases are parser-recognized call names.

Hot aliases:

| Alias | Call |
| --- | --- |
| `C` | `io.println` |
| `A` | `fs.write_text` |
| `E` | `json.query` |
| `F` | `json.query_bool` |
| `X` | `json.query_int` |
| `0` | `json.query_len` |
| `P` | `json.query_at` |
| `S` | `json.contains` |
| `T` | `json.len` |
| `U` | `crypto.sha256_hex` |
| `1` | `fs.is_file` |
| `2` | `fs.write_json_atomic` |
| `3` | `json.valid` |
| `4` | `crypto.sha256_file_hex` |
| `5` | `fs.exists` |
| `6` | `json.query_has` |
| `7` | `json.compact` |
| `8` | `fs.read_json` |
| `9` | `path.normalize` |
| `A0` | `json.query_kind` |
| `C0` | `fs.write_base64` |
| `E0` | `json.has` |
| `F0` | `json.kind` |
| `H0` | `json.query_keys_json` |
| `I0` | `fs.read_base64_tail` |
| `J0` | `fs.read_base64_range` |
| `M0` | `fs.read_base64` |
| `N0` | `fs.glob` |
| `P0` | `fs.find` |
| `S0` | `fs.walk_stat_json` |
| `T0` | `fs.list_stat_json` |
| `U0` | `fs.walk_json` |
| `X0` | `fs.modified` |

Pack alias roots:

| Root | Pack |
| --- | --- |
| `I` | `io` |
| `F` | `fs` |
| `C` | `crypto` |
| `E` | `env` |
| `X` | `process` |
| `A` | `cli` |
| `J` | `json` |
| `S` | `str` |
| `P` | `path` |
| `U` | `url` |
| `T` | `time` |
| `Q` | `tcp` |
| `L` | `map` |
| `H` | `http` |
| `M` | `heap` |
| `N` | `async` |

Frequently used generated aliases:

| Alias | Call |
| --- | --- |
| `Ia` / `Ib` / `Ic` / `Id` | `io.println` / `io.print` / `io.eprintln` / `io.read_line` |
| `Aa`..`Ag` | `cli.argc`, `cli.arg`, `cli.has`, `cli.value`, `cli.value_or`, `cli.args_json`, `cli.parse_json` |
| `Ta` / `Tb` / `Tc` / `Td` | `time.now`, `time.now_ms`, `time.iso_utc`, `time.sleep_ms` |
| `Qa` / `Qb` / `Qc` | `tcp.listen`, `tcp.connect`, `tcp.serve_text` |
| `La` | `map.new` |
| `Na` / `Nb` | `async.cancel`, `async.detach` |
| `Ha` / `Hb` / `Hc` / `Hd` | `http.get`, `http.post`, `http.get_json`, `http.post_json` |
| `Ma` / `Mb` | `heap.alloc`, `heap.free` |
| `Sa` / `Sb` / `Sg` / `Sh` / `Si` / `Sj` / `Sm` / `Sp` / `Sq` / `Sr` / `Ss` / `St` / `Su` | `str.len`, `str.contains`, `str.trim`, `str.upper`, `str.lower`, `str.concat`, `str.slice`, `str.from_i64`, `str.parse_i64`, `str.parse_i32`, `str.token`, `str.line`, `str.token_upper` |

Generated aliases use lowercase base-26 operation indexes after the root: `a` is op 0, `b` op 1, ..., `z` op 25, `aa` op 26.

## Guardrails

- Always run `ax check <file.ax>` after editing.
- Run `ax graph <file.ax>` when effects, packs, or call expansion matter.
- Run `ax build` or `ax run` before claiming executable behavior.
- Do not emit old syntax: `use`, `fn`, `let`, `return`, `if`, `else`, `while`, `server`, `tcp :3000`, `true`, or `false`.
- Prefer compact aliases or proven object methods for std calls. Some expanded std roots are reserved by the parser and may not parse as source.
