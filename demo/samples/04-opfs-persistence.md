# OPFS 持久化

Origin Private File System（OPFS）是浏览器提供的私有文件系统，网页可在其中读写文件，数据持久化跨刷新保留。

## 与 IndexedDB

OPFS 提供同步访问句柄（createSyncAccessHandle），适合 Worker 内同步读写。IndexedDB 是异步的，作为 OPFS 不可用时的降级方案。

## Worker

OPFS 的同步句柄仅在 Dedicated Worker 内可用，主线程只能用异步 API。Vane 的 VaneWorker 在 Worker 内同步调用 core，经 postMessage Promise 边界与主线程通信。
