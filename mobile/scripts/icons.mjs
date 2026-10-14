// Derive native launcher assets from the repository's existing SVG logo.
// macOS: `node scripts/icons.mjs` (sips is bundled with macOS).
import { readFile, writeFile, mkdir, rm } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
const temp = join(tmpdir(), `tandem-icons-${process.pid}`);
await mkdir(temp, { recursive: true });
try {
  let svg = await readFile('../assets/icon.svg', 'utf8');
  svg = svg.replace('<rect x="100" y="100" width="824" height="824" rx="186"', '<rect x="0" y="0" width="1024" height="1024" rx="0"');
  svg = svg.replace(/<rect x="101\.500"[^>]+\/>/, '');
  await writeFile(join(temp, 'icon.svg'), svg);
  execFileSync('sips', ['-s', 'format', 'png', join(temp, 'icon.svg'), '--out', join(temp, 'icon.png')], { stdio: 'ignore' });
  // App Store icons must have no alpha channel, even with an opaque SVG background.
  execFileSync('sips', ['-s', 'format', 'jpeg', '-s', 'formatOptions', '100', join(temp, 'icon.png'), '--out', join(temp, 'opaque.jpg')], { stdio: 'ignore' });
  execFileSync('sips', ['-s', 'format', 'png', join(temp, 'opaque.jpg'), '--out', join(temp, 'icon.png')], { stdio: 'ignore' });
  execFileSync('sips', ['-z', '1024', '1024', join(temp, 'icon.png'), '--out', 'ios/App/App/Assets.xcassets/AppIcon.appiconset/AppIcon-512@2x.png'], { stdio: 'ignore' });
  for (const [density, size] of Object.entries({ mdpi: 48, hdpi: 72, xhdpi: 96, xxhdpi: 144, xxxhdpi: 192 })) {
    for (const name of ['ic_launcher', 'ic_launcher_round', 'ic_launcher_foreground']) {
      const edge = name.endsWith('foreground') ? size * 2.25 : size;
      execFileSync('sips', ['-z', String(edge), String(edge), join(temp, 'icon.png'), '--out', `android/app/src/main/res/mipmap-${density}/${name}.png`], { stdio: 'ignore' });
    }
  }
} finally { await rm(temp, { recursive: true, force: true }); }
