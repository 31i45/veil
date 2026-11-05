# Veil GitHub Push - Spec

## Summary
Push Veil v1.0.0 到 GitHub repo 31i45/veil。用 git 命令（不用 gh）。用户本地密钥已配。包含：clean 工作树 → 配 remote → push main → push tag v1.0.0 → 等 GitHub Actions 跑 ci.yml + release.yml → 验证 Release 创建成功。

## Constraints
- 只用 git 命令，不用 gh、不用 curl、不用 API token
- 不操作仓库 Settings（description、topics 等用户后续手动配）
- 如果 GitHub repo 31i45/veil 不存在，`git push` 会返回 404，此时告知用户需先在 GitHub 网页创建空 repo

## AC (all rule)
- AC-1: `git push -u origin main` exit 0，远程可见所有 commit
- AC-2: `git push origin v1.0.0` exit 0，tag 在 GitHub 可见
- AC-3: GitHub Actions ci.yml 触发，test + clippy 绿
- AC-4: GitHub Actions release.yml 触发，veil-windows-x86_64.zip 上传，Release 自动创建
