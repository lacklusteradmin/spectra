# 脚本说明

这里有 24 个脚本或工具配置文件。日常优先通过 Makefile 运行：

```sh
make verify       # 格式、Rust 静态检查、Rust 测试、CLI 测试、iOS 模拟器测试
make test-cli     # 只跑命令行验收
make check-ui     # 检查界面样式和 SVG 图标格式
```

## CLI 测试

CLI 指命令行程序 `spectra`。这些测试创建临时数据库；需要节点回复时，
在本机 `127.0.0.1` 启动模拟节点，不向真实链广播交易。
测试结束后清理数据库和节点。运行环境需要允许绑定本机端口。

| 文件 | 实际做什么 |
|---|---|
| `cli-acceptance.sh` | 总入口：构建或使用指定的 `spectra`，检查命令输出、退出码、参数拒绝等，再运行下面五组集成测试。 |
| `cli-wallets.py` | 钱包导入、助记词和附加口令处理、非法派生参数拒绝、自动命名、收款地址和密码校验。 |
| `cli-portfolio.py` | 余额刷新及读取失败时保留余额；网络与代币区分；资产估值、缺失报价和汇率；投资组合开关的持久化及总值变化；价格和资产涨跌提醒。 |
| `cli-history.py` | 比特币历史完整翻页和重复刷新不重复保存；本地历史分页、搜索、排序、去重、来源名称；坏记录拒绝；单笔交易状态复查。 |
| `cli-send.py` | 发送预览、给自己转账确认、手续费拒绝、取消和替换交易、网络不匹配拒绝；错误密码不广播，正确密码广播一次并保存对应交易。 |
| `cli-diagnostics.py` | 断网刷新结果、后台维护策略、故障和恢复日志；诊断使用选中的网络并拒绝错误链节点；质押验证者查询使用配置的节点。 |
| `cli-assertions.sh` | Shell 测试共用的检查函数：核对退出码、输出内容，并累计通过和失败数量。由其他脚本引用。 |
| `test-cli-assertions.sh` | 检查上述检查函数本身，防止命令失败却被算作通过。 |

五个 Python 文件使用 Python 标准库 `unittest`，每个场景单独报告结果，
一个场景失败后仍继续执行同文件的其他场景。无需安装第三方 Python 包。
可单独运行一个文件或一个场景：

```sh
python3 scripts/cli-portfolio.py
python3 scripts/cli-history.py target/debug/spectra
python3 scripts/cli-send.py target/debug/spectra SendTests.test_password_protected_broadcast
```

不要用 `python -O` 或 `PYTHONOPTIMIZE`：这些测试依赖断言，禁用断言时会拒绝运行。
节点模拟能检查请求、错误处理和存储结果，不能证明真实链接受了交易。

## 编译和接口生成

| 文件 | 实际做什么 |
|---|---|
| `build-ios.sh` | 编译供 iPhone 真机和模拟器使用的 Rust 库，合并模拟器架构。可加 `--release`。 |
| `build-android.sh` | 编译 Android 各架构的 Rust 库，复制到 `jniLibs`。需要 Android NDK 和 cargo-ndk，可加 `--release`。 |
| `bindgen-ios.sh` | 从编译好的 Rust 库生成 Swift 调用接口，并应用项目的生成修正。不要手改生成结果。 |
| `bindgen-android.sh` | 从编译好的 Rust 库生成 Kotlin 调用接口。 |
| `ios-rust-build-env.sh` | 供构建脚本引用：统一最低 iOS 版本，版本变化时清理不匹配的 iOS 编译缓存。 |

## 代码和资源检查

这些源码扫描脚本用于定位问题；判断无用代码时仍需核对动态调用等情况。
它们不全在 `make verify` 中，按需直接运行。

| 文件 | 实际做什么 |
|---|---|
| `count-exports.sh` | 统计 Rust 向 Swift 开放的函数和方法数量。 |
| `unreachable-exports.sh` | 找已开放给前端、但 Swift 和 CLI 都找不到调用者的 Rust 接口。 |
| `uncalled-core-fns.sh` | 找 Rust 核心中声明公开、却找不到调用者的函数。 |
| `unused-strings.sh` | 找没有使用的文案和不同语言之间不一致的翻译键。 |
| `swift-shell-literals.sh` | 找 Swift 中写死的链名称和金额精度，防止界面再次自行决定业务规则。 |
| `check-design-tokens.sh` | 找绕过统一样式、直接写在 Swift 界面里的圆角和透明度等数值。 |

## 图标

| 文件 | 实际做什么 |
|---|---|
| `normalize-icons.sh` | 将币种和法币 SVG 整理为统一格式；`--check` 只检查，不改文件。 |
| `svgo.config.mjs` | 上述脚本使用的 SVG 整理规则，是配置文件。 |
| `export-swift-icons.sh` | 把 `icons/` 中的源图标转换、同步到 Xcode 图片资源目录；应用图标转为 PNG。 |

修改图标后运行：

```sh
scripts/normalize-icons.sh && scripts/export-swift-icons.sh
```

## 生成测试参考数据

这两个脚本使用独立 SDK 生成参考结果，供 Rust 测试核对。
平时运行测试不需要重新生成。各文件开头注明 SDK 版本和安装、运行方法。

| 文件 | 实际做什么 |
|---|---|
| `generate-protocol-vectors.cjs` | 用 TON、NEAR SDK 生成协议测试参考数据。 |
| `generate-send-audit-vectors.cjs` | 用 Sui、Aptos、Solana、Tron 等 SDK 生成地址、交易和签名参考数据。 |

## 旧测试去哪里了

以前按整改批次命名的 13 个文件已合成 5 个按功能命名的文件。
旧规划和历史变更记录中的文件名表示当时的路径，可按下表找到当前测试。

| 旧文件 | 当前位置 |
|---|---|
| `cli-stage3.sh` | 故障恢复在 `cli-diagnostics.py`，余额刷新在 `cli-portfolio.py`；不存在的交易重播检查在总入口。删除空钱包结构断言。 |
| `cli-stage3-followup.sh` | 改名持久化在总入口；投资组合开关在 `cli-portfolio.py`；维护策略在 `cli-diagnostics.py`；交易复查和发送分别归入对应文件。删除空价格缓存和空维护范围的独立检查。 |
| `cli-shell-ownership.py` | 自动命名和收款归钱包；涨跌提醒归资产；质押节点查询归诊断。 |
| `cli-shell-boundary.py` | 导入参数归钱包；断网刷新归诊断。 |
| `cli-shell-five-fixes.py` | 密码校验归钱包；网络诊断归诊断；加密钱包发送归发送。 |
| `cli-projection-boundary.py` | 估值归资产；历史分页、搜索和去重归历史。 |
| `cli-balance-refresh.py`、`cli-network-token-identity.py` | `cli-portfolio.py`。 |
| `cli-bitcoin-history.py`、`cli-history-corruption.py`、`cli-history-source.py`、`cli-transaction-recheck.py` | `cli-history.py`。 |
| `cli-owned-send.py` | 发送相关场景在 `cli-send.py`；价格提醒在 `cli-portfolio.py`。 |

CLI 集成测试保留跨进程和数据库结果检查；纯函数的细节由 Rust 单元测试负责。
不再为了某次整改新增一个文件，也不以累计检查数量衡量覆盖质量。
