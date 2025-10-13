// demo 合成语料：1 万条英文维基摘要（确定性 PRNG，跨机器可复现）。
// 注意：非真实维基语料。demo 重点在 hybrid/vector/text 三列排序对比与代码量，
// 真实语料需联网下载；此处用确定性生成保证 demo 可离线复现。

import { pathToFileURL } from 'node:url';

// mulberry32 PRNG（确定性，seed=42）
function mulberry32(seed) {
  let a = seed >>> 0;
  return function next() {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

// 词库（约 200 词，覆盖科技/历史/地理/生物/艺术等领域）
const WORDS = [
  // 科技
  'algorithm', 'quantum', 'neural', 'network', 'circuit', 'compiler', 'kernel',
  'protocol', 'cipher', 'database', 'tensor', 'gradient', 'embedding', 'token',
  'vector', 'matrix', 'recursion', 'cache', 'runtime', 'binary', 'syntax',
  'module', 'function', 'variable', 'boolean', 'integer', 'register', 'thread',
  // 历史
  'renaissance', 'empire', 'dynasty', 'monarchy', 'revolution', 'treaty',
  'colonial', 'feudal', 'crusade', 'reformation', 'enlightenment', 'republic',
  'senate', 'consul', 'pharaoh', 'medieval', 'ancient', 'classical', 'byzantine',
  // 地理
  'europe', 'asia', 'africa', 'america', 'ocean', 'mountain', 'river', 'valley',
  'desert', 'island', 'peninsula', 'plateau', 'glacier', 'estuary', 'savanna',
  'tundra', 'archipelago', 'continent', 'latitude', 'equator',
  // 生物
  'ecosystem', 'species', 'genome', 'enzyme', 'protein', 'cell', 'tissue',
  'organ', 'membrane', 'bacteria', 'fungi', 'coral', 'mammal', 'reptile',
  'habitat', 'mutation', 'evolution', 'symbiosis', 'pollination', 'predator',
  // 艺术
  'symphony', 'sonata', 'fresco', 'sculpture', 'ballet', 'opera', 'theatre',
  'poetry', 'stanza', 'canvas', 'portrait', 'mural', 'melody', 'harmony',
  'rhythm', 'chorus', 'baroque', 'rococo', 'impressionism',
  // 通用名词/形容词/动词
  'dynamics', 'mechanism', 'structure', 'process', 'system', 'theory',
  'method', 'concept', 'principle', 'phenomenon', 'influence', 'emergence',
  'integration', 'transformation', 'development', 'foundation', 'expansion',
  'decline', 'restoration', 'innovation', 'complex', 'fundamental', 'remarkable',
  'significant', 'gradual', 'sudden', 'widespread', 'ancient', 'modern',
  'classical', 'traditional', 'revolutionary', 'influence', 'shape', 'reflect',
  'emerge', 'integrate', 'dominate', 'transform', 'sustain', 'accelerate',
];

// 句式池（≥6 种）
const TEMPLATES = [
  (p) => `The ${p.topic} is a ${p.adj} ${p.noun} that ${p.verb} ${p.topic2}.`,
  (p) => `Historically, ${p.topic} emerged in ${p.year} as a response to ${p.topic2}.`,
  (p) => `Modern ${p.topic} integrates ${p.topic3}, ${p.topic2}, and ${p.topic}.`,
  (p) => `Researchers note that ${p.topic} influences ${p.topic2} via ${p.adj} mechanisms.`,
  (p) => `The study of ${p.topic} reveals a ${p.adj} interplay between ${p.topic2} and ${p.topic3}.`,
  (p) => `Across ${p.region}, ${p.topic} has shaped both ${p.topic2} and ${p.topic3} since ${p.year}.`,
  (p) => `A ${p.adj} feature of ${p.topic} is its capacity to ${p.verb} ${p.topic2} under ${p.adj} conditions.`,
];

const ADJS = ['complex', 'fundamental', 'remarkable', 'significant', 'gradual', 'sudden', 'widespread', 'classical', 'traditional', 'revolutionary'];
const NOUNS = ['phenomenon', 'structure', 'process', 'system', 'theory', 'method', 'concept', 'principle', 'mechanism', 'framework'];
const VERBS = ['influence', 'shape', 'reflect', 'emerge', 'integrate', 'dominate', 'transform', 'sustain', 'accelerate'];
const REGIONS = ['europe', 'asia', 'africa', 'america', 'the mediterranean', 'the tropics', 'the arctic'];

function pick(rng, arr) {
  return arr[Math.floor(rng() * arr.length)];
}

function cap(s) {
  return s.charAt(0).toUpperCase() + s.slice(1);
}

/**
 * 生成 count 条确定性英文维基摘要。
 * @param {number} count 默认 10000
 * @param {number} seed 默认 42
 * @returns {Array<{id:string,title:string,text:string}>}
 */
export function generateWikiAbstracts(count = 10000, seed = 42) {
  const rng = mulberry32(seed);
  const out = new Array(count);
  for (let i = 0; i < count; i++) {
    const id = 'wiki-' + String(i).padStart(5, '0');
    const t1 = pick(rng, WORDS);
    const t2 = pick(rng, WORDS);
    const title = `${cap(t1)} ${cap(pick(rng, WORDS))} of ${cap(t2)} ${cap(pick(rng, WORDS))}`;
    // 3~5 句
    const nSent = 3 + Math.floor(rng() * 3);
    const sentences = [];
    for (let s = 0; s < nSent; s++) {
      const tpl = pick(rng, TEMPLATES);
      sentences.push(tpl({
        topic: pick(rng, WORDS),
        topic2: pick(rng, WORDS),
        topic3: pick(rng, WORDS),
        adj: pick(rng, ADJS),
        noun: pick(rng, NOUNS),
        verb: pick(rng, VERBS),
        year: 1500 + Math.floor(rng() * 500),
        region: pick(rng, REGIONS),
      }));
    }
    const text = sentences.join(' ');
    out[i] = { id, title, text };
  }
  return out;
}

// ---- inline smoke 自检 ----
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const d1 = generateWikiAbstracts(100);
  const d2 = generateWikiAbstracts(10000, 42);
  const d2b = generateWikiAbstracts(10000, 42);
  const asserts = [];
  asserts.push(['len=100', d1.length === 100]);
  asserts.push(['deterministic-10k', JSON.stringify(d2) === JSON.stringify(d2b)]);
  asserts.push(['id0=wiki-00000', d2[0].id === 'wiki-00000']);
  asserts.push(['text non-empty >50', d2[0].text.length > 50]);
  const ids = new Set(d2.map((d) => d.id));
  asserts.push(['10000 unique ids', ids.size === 10000]);
  let ok = true;
  for (const [name, pass] of asserts) {
    console.log(`${pass ? 'OK' : 'FAIL'}  ${name}`);
    if (!pass) ok = false;
  }
  process.exit(ok ? 0 : 1);
}
