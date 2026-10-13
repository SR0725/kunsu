---
title: 這個阻塞要採用哪個處理方式？
question_line: 要採用哪個處理方式？
kind: choice
tier: t1
default_action:
  on_expire: noop_log
options:
  - id: recommended
    label: 採用建議方案
    detail: 補上具體做法、影響範圍與理由
    recommended: true
  - id: defer
    label: 暫緩處理
    detail: 保留現狀，補上延後的後果
---
要你決定：補上這次真正卡住的決策。

## 已知事實

寫入自己查過的事實、來源與觀測時間；沒有證據的部分標示未知。

## 我的建議

說清楚建議與理由。補上兩個選項各自的後果，以及不決定時工作停在哪裡。
