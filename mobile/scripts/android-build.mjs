import { existsSync, readdirSync } from 'node:fs';
import { resolve, join } from 'node:path';
import { homedir } from 'node:os';
import { spawnSync } from 'node:child_process';

const env = { ...process.env };
const local = resolve('.tooling');
if (!env.JAVA_HOME && existsSync(join(local, 'jdk'))) {
  const candidate = readdirSync(join(local, 'jdk')).find(name => name.startsWith('jdk-'));
  if (candidate) env.JAVA_HOME = join(local, 'jdk', candidate, process.platform === 'darwin' ? 'Contents/Home' : '');
}
if (!env.ANDROID_HOME) {
  const candidates = [env.ANDROID_SDK_ROOT, join(local, 'android-sdk'), join(homedir(), 'Library/Android/sdk'), join(homedir(), 'Android/Sdk')];
  if (env.LOCALAPPDATA) candidates.push(join(env.LOCALAPPDATA, 'Android/Sdk'));
  env.ANDROID_HOME = candidates.find(path => path && existsSync(path));
}
if (!env.ANDROID_HOME) throw new Error('Install Android Studio/SDK and set ANDROID_HOME, then run this command again.');
if (env.ANDROID_HOME === join(local, 'android-sdk') && !env.GRADLE_USER_HOME) env.GRADLE_USER_HOME = join(local, 'gradle');
const command = process.platform === 'win32' ? 'gradlew.bat' : './gradlew';
const result = spawnSync(command, ['assembleDebug', '--no-daemon'], { cwd: resolve('android'), env, stdio: 'inherit', shell: process.platform === 'win32' });
if (result.error) throw result.error;
process.exitCode = result.status ?? 1;
