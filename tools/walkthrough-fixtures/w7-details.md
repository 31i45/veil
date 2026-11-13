# `<details>` 折叠真机验收

## 1. 默认收起

<details>
<summary>点我展开（默认收起）</summary>

折叠正文第一段：默认状态下不可见。

折叠正文第二段：展开后应完整可见。

- 折叠内列表项一
- 折叠内列表项二

</details>

## 2. 显式 open

<details open>
<summary>显式 open（默认展开）</summary>

这段在显式 open 下应默认可见。

</details>

## 3. 嵌套 Markdown 子块

<details>
<summary>嵌套 Markdown</summary>

### 折叠内三级标题

折叠内段落带 **粗体** 与 `行内代码`。

```rust
// 折叠内代码块
fn folded() {}
```

| 列 A | 列 B |
| --- | --- |
| 1 | 2 |

</details>

## 4. 尾部锚点

本行在全部折叠收起时应紧邻上方；展开后应被推下去。
