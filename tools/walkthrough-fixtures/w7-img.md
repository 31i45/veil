# `<img>` 本地 / 远程 / 缺失真机验收

## 1. 本地图片（应加载）

<img src="veil-logo.png" alt="本地标志">

正文行内本地图：<img src="veil-logo.png" alt="行内标志"> 后接文字。

## 2. 远程图片（应占位 + alt，不发网络请求）

<img src="https://example.com/remote.png" alt="远程图片">

正文行内远程图：<img src="https://example.com/remote2.png" alt="远程行内"> 后接文字。

## 3. 缺失图片（应占位 + alt）

<img src="veil-missing.png" alt="缺失图片">

## 4. 无 alt

<img src="veil-missing2.png">

## 5. 带尺寸属性（属性应被过滤或忽略，不影响渲染）

<img src="veil-logo.png" alt="带宽度" width="320" height="200">

## 6. 尾部锚点

用于确认图片块高度不影响后文。
