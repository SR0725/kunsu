---
title: 這批處理方案請你逐項決定
question_line: 這批方案要怎麼處理？
kind: batch
tier: t1
default_action:
  on_expire: noop_log
items:
  - id: item-a
    title: 第一個待決項目
    context_md: 替換成這一項的真實來源、原文與必要脈絡。
    proposal: 替換成可以直接採用的完整提案。
    editable: true
    default: approve
  - id: item-b
    title: 第二個待決項目
    context_md: 替換成這一項的真實來源、原文與必要脈絡。
    proposal: 替換成可以直接採用的完整提案。
    editable: true
    default: approve
---
要你決定：逐項確認這批同類工作的處理方式。

將每項的原文與可執行提案寫完整。一批放 1–20 項；單項也保留 batch 可編輯草稿。
此範本不含退款、支出或其他錢操作。要對外發送時，正式回答也只能授權這次清楚呈現的項目與內容。
