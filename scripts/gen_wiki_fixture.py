#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""M2-13 离线 fixture 生成器（真实中文维基 corpus + 自然 trap）。

从 zh.wikipedia.org API 抓取真实文章 intro 正文，构造：
- corpus.json：500 篇 {id, title, domain, text}（200~2000 字）
- queries.json：50 查询 {qid, text, type}（≥10 边界歧义）
- qrels.json：由 `cargo run --example gen_qrels --features dict-zh` 生成
  （jieba-lite tokenization-aware：query 作为 jieba 词元出现 = 强匹配）

## 自然 trap 机制（真实维基落地 M1 合成 trap）
对每个边界歧义查询 Q（jieba 单 token，bigram [b1, b2, ...]）：
- 主条目（title=Q）：rel=3。
- trap 条目：真实维基短文章，含某子二元组（如 Q=人工智能 → trap=智能手机含「智能」）
  但不含 Q 本身 → rel=0。trap 文章短（200~500 字）→ bigram BM25 高（长度归一化）→
  挤占 top-10；jieba 整词不命中 → 排序质量高。

所有文章均为真实维基内容（非合成）。qrels 由 jieba-lite 分词决定（词边界内 = 相关）。
"""
from __future__ import annotations

import json
import sys
import time
import urllib.parse
import urllib.request
from pathlib import Path

API = "https://zh.wikipedia.org/w/api.php"
OUT = Path(__file__).resolve().parent.parent / "crates/vane-core/tests/fixtures/wiki_zh"
CACHE = Path(__file__).resolve().parent.parent / "target/wiki_cache.json"
BATCH = 20
MIN_CHARS = 200
MAX_CHARS = 2000
N_DOCS = 500

# -----------------------------------------------------------------------
# 边界歧义查询：(query, primary_title, domain, [trap_titles])
# trap 须含 query 的某子二元组但不含 query（程序校验）。trap 优先选短文章。
# -----------------------------------------------------------------------
BOUNDARY: list[tuple[str, str, str, list[str]]] = [
    ("人工智能", "人工智能", "科技",
     ["智能手机", "智能家居", "智能合约", "人工呼吸", "人工湖", "人工智能伦理"]),
    ("太阳能", "太阳能", "科技",
     ["太阳系", "太阳镜", "太阳风", "太阳神", "太阳花", "太阳雨"]),
    ("计算机科学", "计算机科学", "科技",
     ["计算器", "算盘", "科学方法", "科学革命", "计算机图形学", "计算机工程"]),
    ("进化论", "进化论", "科技",
     ["进化", "化学进化", "演化生物学", "博物学", "分类学", "古生物学"]),
    ("量子力学", "量子力学", "科技",
     ["量子", "量子数", "量子场论", "力学", "流体力学", "固体力学"]),
    ("相对论", "相对论", "科技",
     ["相对性原理", "狭义相对论", "广义相对论", "物理定律", "理论物理", "实验物理"]),
    ("核反应堆", "核反应堆", "科技",
     ["核反应", "核裂变", "核聚变", "反应堆", "核工程", "核电站"]),
    ("无人机", "无人机", "科技",
     ["无人驾驶", "遥感", "航空模型", "飞行器", "导航", "遥控"]),
    ("互联网", "互联网", "科技",
     ["物联网", "局域网", "广域网", "网络协议", "路由器", "网络拓扑"]),
    ("万维网", "万维网", "科技",
     ["网页", "网站", "HTTP", "HTML", "网络浏览器", "超文本"]),
    ("数据中心", "数据中心", "科技",
     ["数据库", "数据挖掘", "数据结构", "大数据", "云存储", "服务器"]),
    ("工业革命", "工业革命", "历史",
     ["工业", "工业化", "制造业", "工厂", "蒸汽机", "纺织业"]),
    ("文艺复兴", "文艺复兴", "历史",
     ["文学", "艺术", "古典主义", "人文主义", "建筑史", "美术史"]),
    ("丝绸之路", "丝绸之路", "历史",
     ["丝绸", "贸易路线", "中亚", "西域", "敦煌", "骆驼"]),
    ("第一次世界大战", "第一次世界大战", "历史",
     ["世界大战", "战争", "军事史", "凡尔赛条约", "欧洲历史", "战壕战"]),
    ("第二次世界大战", "第二次世界大战", "历史",
     ["世界战争", "战役", "军事", "诺曼底登陆", "太平洋战争", "冷战"]),
    ("兵马俑", "兵马俑", "历史",
     ["兵马", "陶俑", "秦朝", "陵墓", "考古学", "秦始皇陵"]),
    ("万里长城", "万里长城", "历史",
     ["长城", "城墙", "防御工事", "边防", "明朝", "烽火台"]),
    ("珠穆朗玛峰", "珠穆朗玛峰", "地理",
     ["山脉", "山峰", "喜马拉雅山脉", "登山", "高原", "冰川"]),
    ("青藏高原", "青藏高原", "地理",
     ["高原", "西藏", "高原气候", "草原", "牧区", "冻土"]),
    ("撒哈拉沙漠", "撒哈拉沙漠", "地理",
     ["沙漠", "沙丘", "绿洲", "干旱", "荒漠化", "游牧"]),
    ("喜马拉雅山脉", "喜马拉雅山脉", "地理",
     ["山脉", "地震带", "板块构造", "造山运动", "冰川", "高原"]),
    ("自然保护区", "自然保护区", "地理",
     ["自然", "生态保护", "野生动物", "生态系统", "生物多样性", "国家公园"]),
    ("京杭大运河", "京杭大运河", "地理",
     ["运河", "航运", "水利工程", "河道", "北京", "杭州"]),
    ("太阳系", "太阳系", "科技",
     ["太阳", "行星", "恒星", "轨道", "天文学", "引力"]),
    # M1-style 边界歧义词：常见子二元组（科学/工程/委员/思想/政治/运动/文学/哲学）
    # 在 corpus 中以 jieba 词元出现 → rel>0；含子二元组但不含整词的文档 → rel=0 trap。
    ("科学家", "科学家", "科技",
     ["科学方法", "科学革命", "科学哲学", "数学", "物理学", "化学"]),
    ("委员会", "委员会", "历史",
     ["委员", "委员长", "国务院", "全国人大", "政协", "议会"]),
    ("思想家", "思想家", "历史",
     ["思想", "思想史", "哲学", "哲学史", "启蒙", "理性"]),
    ("政治家", "政治家", "历史",
     ["政治", "政治学", "政府", "政党", "选举", "外交"]),
    ("运动会", "运动会", "地理",
     ["运动", "运动员", "体育", "足球", "篮球", "奥运"]),
    ("文学家", "文学家", "历史",
     ["文学", "文学史", "诗歌", "小说", "散文", "戏剧"]),
    ("哲学家", "哲学家", "历史",
     ["哲学", "哲学史", "逻辑学", "伦理学", "形而上学", "认识论"]),
]

# 实体查询（2~4 字 title）
ENTITY_TITLES = [
    "唐朝", "宋朝", "明朝", "秦始皇", "汉武帝", "成吉思汗", "孔子", "李白",
    "孙中山", "毛泽东", "长江", "黄河", "太平洋", "北京", "上海", "东京",
    "拿破仑", "电池", "长城", "故宫", "5G", "C语言", "3D打印", "内燃机",
    "信息技术", "信息安全", "HTML",
]

# 背景标题
TECH = [
    "机器学习", "深度学习", "神经网络", "区块链", "量子计算", "云计算",
    "物联网", "半导体", "集成电路", "芯片", "操作系统", "Linux",
    "数据库", "编程语言", "Python", "Java", "JavaScript", "Rust",
    "算法", "数据结构", "编译器", "虚拟现实", "增强现实",
    "网络安全", "密码学", "加密", "比特币", "以太坊",
    "搜索引擎", "互联网", "万维网", "TCP/IP", "路由器", "服务器",
    "量子计算机", "量子纠缠", "黑洞", "原子", "电子",
    "质子", "中子", "DNA", "RNA", "基因", "基因组", "蛋白质", "细胞",
    "干细胞", "病毒", "细菌", "免疫系统", "疫苗", "抗生素",
    "达尔文", "孟德尔", "克隆", "基因编辑", "CRISPR", "自然语言处理",
    "计算机视觉", "语音识别", "机器人", "自动驾驶",
    "纳米技术", "石墨烯", "超导体", "激光", "光纤", "风能",
    "核能", "核聚变", "核裂变", "锂电池", "氢能源",
    "电动机", "发电机", "蒸汽机", "涡轮", "火箭", "卫星",
    "太空探索", "月球", "火星", "银河系", "宇宙学", "大爆炸理论",
    "图灵", "冯·诺伊曼", "香农",
    "微软", "谷歌", "苹果公司", "亚马逊", "腾讯", "阿里巴巴",
    "华为", "百度", "英特尔", "英伟达", "台积电",
    "信息技术", "软件工程", "源代码", "开源软件",
    "强化学习", "决策树", "随机森林", "支持向量机",
    "机器翻译", "聊天机器人",
    "分布式计算", "并行计算", "边缘计算",
    "信息安全", "数字签名", "公钥加密", "哈希函数",
    "固态硬盘", "硬盘", "内存", "中央处理器", "图形处理器",
    "生物信息学", "天体物理学", "中微子", "夸克", "玻色子",
    "化学反应", "催化剂", "高分子", "塑料", "橡胶",
    "地质学", "地震", "火山", "矿物", "宝石",
    "天文学", "光学", "声学", "热力学", "电磁学", "量子场论",
    "生物化学", "分子生物学", "遗传学", "生态学", "动物学", "植物学",
    "气象学", "海洋学", "地理信息系统", "测绘学",
]
HISTORY = [
    "元朝", "清朝", "汉朝", "秦朝", "隋朝",
    "晋朝", "商朝", "周朝", "春秋时期", "战国时期", "三国", "五代十国",
    "启蒙运动", "法国大革命", "美国独立战争",
    "冷战", "十字军东征", "罗马帝国",
    "拜占庭帝国", "奥斯曼帝国", "蒙古帝国", "亚历山大帝国",
    "唐太宗", "武则天", "朱元璋", "康熙帝", "乾隆帝",
    "忽必烈", "李世民", "诸葛亮", "曹操", "孙权",
    "刘备", "岳飞", "文天祥", "郑和", "郑成功", "林则徐",
    "袁世凯", "邓小平", "蒋介石", "老子", "庄子",
    "孟子", "墨子", "韩非子", "孙子", "司马迁", "杜甫",
    "白居易", "苏轼", "王安石", "司马光", "朱熹", "王阳明",
    "鸦片战争", "甲午战争", "辛亥革命", "五四运动",
    "抗日战争", "国共内战", "兵马俑",
    "敦煌", "秦始皇陵", "颐和园", "天坛", "亚历山大大帝",
    "希特勒", "斯大林", "列宁", "马克思", "恩格斯", "丘吉尔",
    "罗斯福", "华盛顿", "林肯", "甘地", "曼德拉",
    "凯撒大帝", "彼得大帝", "维多利亚女王",
    "路易十四", "明治天皇", "丰臣秀吉", "德川家康",
    "夏朝", "新朝", "南北朝", "辽朝", "金朝", "西夏",
    "贞观之治", "开元盛世", "安史之乱", "靖康之变",
    "郑和下西洋", "洋务运动", "戊戌变法",
    "太平天国", "南京条约", "马关条约",
    "长征", "西安事变", "朝鲜战争", "越南战争",
    "古埃及", "巴比伦", "波斯帝国", "玛雅文明",
    "古希腊", "斯巴达", "中世纪", "封建制度", "黑死病", "宗教改革",
    "三十年战争", "七年战争", "普法战争",
    "美国内战", "大萧条",
    "儒家", "道家", "法家", "佛教", "基督教", "伊斯兰教",
    "史记", "资治通鉴", "论语", "道德经",
    "红楼梦", "西游记", "水浒传", "三国演义",
    "莎士比亚", "歌德", "托尔斯泰", "雨果", "鲁迅",
    "安史之乱", "靖康之变", "崖山海战", "洋务运动", "戊戌变法",
    "义和团运动", "太平天国", "中法战争", "庚子赔款",
    "北洋政府", "军阀割据", "南京大屠杀", "西安事变",
    "朝鲜战争", "越南战争", "海湾战争", "冷战",
    "古埃及", "巴比伦", "亚述", "波斯帝国", "玛雅文明", "印加帝国",
    "阿兹特克", "古希腊", "斯巴达", "雅典民主",
    "中世纪", "封建制度", "骑士", "城堡", "黑死病", "宗教改革",
    "三十年战争", "七年战争", "克里米亚战争", "普法战争",
    "美国内战", "大萧条", "水门事件",
    "儒家", "道家", "法家", "佛教", "基督教", "伊斯兰教",
    "印度教", "犹太教", "禅宗",
    "史记", "汉书", "资治通鉴", "四书五经", "论语", "道德经",
    "红楼梦", "西游记", "水浒传",
]
GEO = [
    "长江", "黄河", "珠江", "黑龙江", "雅鲁藏布江", "湄公河",
    "青海湖", "鄱阳湖", "洞庭湖", "太湖",
    "昆仑山脉", "天山", "秦岭", "长白山", "武夷山", "黄山", "庐山",
    "内蒙古高原", "黄土高原", "云贵高原", "塔里木盆地",
    "准噶尔盆地", "柴达木盆地", "四川盆地", "塔克拉玛干沙漠", "戈壁",
    "亚马孙雨林", "刚果盆地", "安第斯山脉", "落基山脉",
    "阿尔卑斯山", "乌拉尔山脉", "大堡礁",
    "死海", "里海", "贝加尔湖", "维多利亚湖",
    "北冰洋", "印度洋", "大西洋", "日本海", "南海",
    "东海", "黄海", "渤海", "马六甲海峡", "直布罗陀海峡",
    "巴拿马运河", "苏伊士运河", "京杭大运河",
    "广州", "深圳", "成都", "重庆", "武汉", "南京", "杭州", "西安",
    "洛阳", "苏州", "桂林", "丽江", "拉萨",
    "乌鲁木齐", "香港", "澳门", "台北", "首尔", "平壤",
    "新加坡", "曼谷", "吉隆坡", "雅加达", "马尼拉", "河内",
    "仰光", "孟买", "新德里", "迪拜", "伊斯坦布尔",
    "开罗", "莫斯科", "巴黎", "伦敦", "柏林",
    "罗马", "马德里", "雅典", "维也纳", "布拉格", "斯德哥尔摩",
    "阿姆斯特丹", "布鲁塞尔", "华沙", "基辅", "圣彼得堡",
    "纽约", "洛杉矶", "旧金山", "芝加哥", "多伦多", "温哥华",
    "墨西哥城", "布宜诺斯艾利斯", "里约热内卢", "圣保罗",
    "悉尼", "墨尔本", "南极洲", "格陵兰", "冰岛",
    "尼罗河", "多瑙河", "莱茵河", "伏尔加河",
    "密西西比河", "亚马逊河", "刚果河",
    "富士山", "高加索山脉",
    "纳米布沙漠", "阿塔卡马沙漠",
    "地中海", "加勒比海", "黑海", "红海",
    "台湾海峡", "好望角", "台湾岛", "海南岛",
    "马达加斯加", "斯里兰卡", "爪哇岛", "苏门答腊",
    "气候", "季风", "温室效应", "全球变暖",
    "生态系统", "热带雨林", "草原", "苔原", "湿地", "珊瑚礁", "冰川",
]


def fetch_batch(titles: list[str]) -> dict[str, str]:
    params = {
        "action": "query",
        "redirects": "1",
        "prop": "extracts",
        "explaintext": "1",
        "exintro": "1",
        "exsectionformat": "plain",
        "format": "json",
        "titles": "|".join(titles),
    }
    url = API + "?" + urllib.parse.urlencode(params)
    data = None
    for attempt in range(6):
        req = urllib.request.Request(url, headers={"User-Agent": "vane-m2-fixture/1.0 (offline generator; contact: dev)"})
        try:
            with urllib.request.urlopen(req, timeout=30) as resp:
                data = json.loads(resp.read().decode("utf-8"))
            break
        except urllib.error.HTTPError as e:
            if e.code == 429:
                wait = 10 * (attempt + 1)
                print(f"  429, retry {wait}s", file=sys.stderr)
                time.sleep(wait)
                continue
            raise
        except Exception as e:
            if attempt < 5:
                print(f"  err: {e}, retry 5s", file=sys.stderr)
                time.sleep(5)
                continue
            raise
    if data is None:
        return {}
    query = data.get("query", {})
    norm = {n["to"]: n["from"] for n in query.get("normalized", [])}
    redir = {r["to"]: r["from"] for r in query.get("redirects", [])}
    out: dict[str, str] = {}
    for _pid, p in query.get("pages", {}).items():
        title = p.get("title", "")
        extract = p.get("extract", "")
        if extract:
            out[title] = extract
            if title in redir:
                out[redir[title]] = extract
            if title in norm:
                out[norm[title]] = extract
    return out


def clean(extract: str) -> str:
    lines = [ln.strip() for ln in extract.splitlines() if ln.strip()]
    text = "".join(lines)
    if len(text) > MAX_CHARS:
        text = text[:MAX_CHARS]
    return text


def main() -> int:
    titled: list[tuple[str, str]] = []
    seen = set()

    def add(title: str, dom: str):
        if title and title not in seen:
            seen.add(title)
            titled.append((title, dom))

    # 边界查询 primary + trap
    for query, primary, dom, traps in BOUNDARY:
        add(primary, dom)
        for tr in traps:
            add(tr, dom)

    for t in ENTITY_TITLES:
        dom = "历史" if t in ("唐朝", "宋朝", "明朝", "秦始皇", "汉武帝", "成吉思汗",
                          "孔子", "李白", "孙中山", "毛泽东", "拿破仑") else "地理" if t in (
                          "长江", "黄河", "太平洋", "北京", "上海", "东京", "长城",
                          "故宫") else "科技"
        add(t, dom)

    for t in TECH:
        add(t, "科技")
    for t in HISTORY:
        add(t, "历史")
    for t in GEO:
        add(t, "地理")

    print(f"curated titles: {len(titled)}")

    # 抓取（带缓存）
    cache: dict[str, str] = {}
    if CACHE.exists():
        try:
            cache = json.loads(CACHE.read_text(encoding="utf-8"))
            print(f"loaded cache: {len(cache)}")
        except Exception:
            cache = {}
    fetched: dict[str, tuple[str, str]] = {}
    for t, dom in titled:
        if t in cache:
            c = clean(cache[t])
            if len(c) >= MIN_CHARS:
                fetched[t] = (c, dom)
    print(f"from cache: {len(fetched)}")

    for i in range(0, len(titled), BATCH):
        batch = titled[i:i + BATCH]
        titles = [t for t, _ in batch if t not in cache]
        if not titles:
            continue
        res = fetch_batch(titles)
        for t in titles:
            if t in res:
                cache[t] = res[t]
        for t, dom in batch:
            txt = res.get(t)
            if txt:
                c = clean(txt)
                if len(c) >= MIN_CHARS:
                    fetched[t] = (c, dom)
        print(f"  batch {i//BATCH+1}: fetched={len(fetched)}")
        CACHE.parent.mkdir(parents=True, exist_ok=True)
        CACHE.write_text(json.dumps(cache, ensure_ascii=False), encoding="utf-8")
        time.sleep(2)

    print(f"total fetched: {len(fetched)}")
    if len(fetched) < N_DOCS:
        print(f"ERROR: only {len(fetched)}", file=sys.stderr)
        return 1

    # 优先保留 boundary primary + trap + entity，其余填充
    priority = set()
    for _q, primary, _dom, traps in BOUNDARY:
        priority.add(primary)
        for tr in traps:
            priority.add(tr)
    for t in ENTITY_TITLES:
        priority.add(t)

    # trap 校验：trap 文档须不含 query（否则不是 trap）
    valid_traps = set()
    for query, _primary, _dom, traps in BOUNDARY:
        for tr in traps:
            if tr in fetched:
                text = fetched[tr][0]
                if query not in text:
                    valid_traps.add(tr)  # 真 trap
                # 含 query 的 trap 仍保留为普通文档（非 trap）
    print(f"valid traps (contain sub-bigram, not query): {len(valid_traps)}")

    pri_docs = [(t, fetched[t]) for t in priority if t in fetched]
    other_docs = [(t, fetched[t]) for t in fetched if t not in priority]
    selected = (pri_docs + other_docs)[:N_DOCS]

    dom_count = {"科技": 0, "历史": 0, "地理": 0}
    for _t, (_txt, dom) in selected:
        dom_count[dom] += 1
    print(f"selected {len(selected)}: {dom_count}")
    assert len(selected) >= N_DOCS
    assert all(v >= 30 for v in dom_count.values()), f"领域覆盖不足 {dom_count}"

    # corpus
    corpus = []
    title_to_id: dict[str, str] = {}
    for idx, (title, (text, dom)) in enumerate(selected):
        doc_id = f"w{idx+1:03d}"
        full = f"{title}。{text}"
        if len(full) > MAX_CHARS:
            full = full[:MAX_CHARS]
        corpus.append({"id": doc_id, "title": title, "domain": dom, "text": full})
        title_to_id[title] = doc_id

    # queries
    queries = []
    used = set()
    for query, primary, _dom, _traps in BOUNDARY:
        if primary in title_to_id:
            queries.append({"text": query, "type": "boundary"})
            used.add(query)
    for t in ENTITY_TITLES:
        if t in title_to_id and t not in used:
            queries.append({"text": t, "type": "entity"})
            used.add(t)
    if len(queries) < 50:
        for d in corpus:
            if len(queries) >= 50:
                break
            t = d["title"]
            if t not in used and 2 <= len(t) <= 4:
                queries.append({"text": t, "type": "entity"})
                used.add(t)
    queries = queries[:50]
    queries_out = [{"qid": f"q{i+1:02d}", "text": q["text"], "type": q["type"]} for i, q in enumerate(queries)]
    assert len(queries_out) == 50, f"need 50, got {len(queries_out)}"
    n_boundary = sum(1 for q in queries_out if q["type"] == "boundary")
    print(f"queries: {len(queries_out)}, boundary: {n_boundary}")
    assert n_boundary >= 10

    # 写文件（qrels 由 gen_qrels.rs 生成）
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / "corpus.json").write_text(json.dumps(corpus, ensure_ascii=False), encoding="utf-8")
    (OUT / "queries.json").write_text(json.dumps(queries_out, ensure_ascii=False, indent=0), encoding="utf-8")
    # 临时 qrels（substring-based，将被 gen_qrels.rs 覆盖）
    qrels_tmp: dict[str, dict[str, int]] = {}
    for q in queries_out:
        qt = q["text"]
        scored = []
        for d in corpus:
            if d["title"] == qt:
                scored.append((d["id"], 3, 999))
            else:
                cnt = d["text"].count(qt)
                if cnt >= 3:
                    scored.append((d["id"], 2, cnt))
                elif cnt >= 1:
                    scored.append((d["id"], 1, cnt))
        scored.sort(key=lambda x: (-x[1], -x[2]))
        qrels_tmp[q["qid"]] = {d: r for d, r, _ in scored[:10] if r > 0}
    (OUT / "qrels.json").write_text(json.dumps(qrels_tmp, ensure_ascii=False, indent=0), encoding="utf-8")

    total_size = sum((OUT / f).stat().st_size for f in ["corpus.json", "queries.json", "qrels.json"])
    print(f"fixture size: {total_size/1024:.1f} KB ({total_size/1024/1024:.2f} MB)")

    # trap 统计
    for query, primary, _dom, traps in BOUNDARY:
        if primary not in title_to_id:
            continue
        n_trap = sum(1 for tr in traps if tr in title_to_id and query not in fetched.get(tr, ("", ""))[0])
        n_sub = sum(1 for d in corpus if d["title"] != query and query not in d["text"]
                    and any(query[i:i+2] in d["text"] for i in range(len(query)-1)))
        print(f"  trap {query}: explicit_traps={n_trap}, corpus_subbigram_docs={n_sub}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
