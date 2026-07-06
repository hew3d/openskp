const fs = require('fs'), path = require('path');
const yaml = require('js-yaml');
const ksc = require('kaitai-struct-compiler');
const ksyPath = process.argv[2], outDir = process.argv[3] || 'tools/gen';
const ksy = yaml.load(fs.readFileSync(ksyPath, 'utf8'));
fs.mkdirSync(outDir, { recursive: true });
ksc.compile('python', ksy, null, false).then(files => {
  for (const [fn, content] of Object.entries(files)) {
    fs.writeFileSync(path.join(outDir, fn), content);
    console.log('wrote', fn, content.length, 'bytes');
  }
}).catch(e => { console.error('COMPILE ERROR:', e && e.message || e); process.exit(1); });
