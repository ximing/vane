# 向量检索与混合搜索

向量检索（Vector Search）是信息检索的一种范式，将文本、图像等内容映射为高维向量，通过向量相似度（如 cosine、L2）排序。

## 混合搜索

混合搜索（Hybrid Search）融合向量检索与词项检索（BM25 / TF-IDF），兼顾语义相似与精确匹配。RRF（Reciprocal Rank Fusion）是常用的融合策略。

## 倒排索引

词项检索依赖倒排索引（Inverted Index），将词项映射到文档列表。BM25 基于词频和文档长度归一化打分。
