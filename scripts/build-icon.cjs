// Development-only: npm install --no-save --package-lock=false sharp@0.35.5
// Run from the repository root: node scripts/build-icon.cjs
// The SVG is the sole artwork source. No upstream images are read.
const fs = require('node:fs/promises');
const path = require('node:path');
const sharp = require('sharp');

async function main() {
  const assets = path.resolve(__dirname, '../assets');
  const source = await fs.readFile(path.join(assets, 'icon.svg'));
  const sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256];
  const images = await Promise.all(sizes.map(size =>
    sharp(source, { density: 384 }).resize(size, size).png().toBuffer()
  ));
  // Windows Vista+ ICO files accept PNG frames, including alpha transparency.
  const header = Buffer.alloc(6 + 16 * sizes.length);
  header.writeUInt16LE(1, 2);
  header.writeUInt16LE(sizes.length, 4);
  let offset = header.length;
  sizes.forEach((size, index) => {
    const entry = 6 + 16 * index;
    header[entry] = header[entry + 1] = size === 256 ? 0 : size;
    header.writeUInt16LE(1, entry + 4);
    header.writeUInt16LE(32, entry + 6);
    header.writeUInt32LE(images[index].length, entry + 8);
    header.writeUInt32LE(offset, entry + 12);
    offset += images[index].length;
  });
  await fs.writeFile(path.join(assets, 'icon.ico'), Buffer.concat([header, ...images]));
  await fs.writeFile(path.join(assets, 'icon.png'), images.at(-1));
}

main().catch(error => { console.error(error); process.exitCode = 1; });
