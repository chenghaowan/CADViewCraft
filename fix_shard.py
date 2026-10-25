# -*- coding: utf-8 -*-
# 删除 icon_uri 残片(从 "fn icon_uri" 行到 "/// 质量属性" 注释前)
p = 'src/app.rs'
lines = open(p, encoding='utf-8').readlines()
start = end = None
for i, l in enumerate(lines):
    if "fn icon_uri" in l and start is None:
        start = i
    if start is not None and "/// 质量属性" in l:
        end = i
        break
assert start is not None and end is not None, (start, end)
print("removing lines", start + 1, "to", end)
del lines[start:end]
open(p, 'w', encoding='utf-8', newline='').writelines(lines)
print("done")
