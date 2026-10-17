import {test} from 'node:test';import assert from 'node:assert/strict';
import {nativeMagic} from './verify-bundle.mjs';
test('packaging rejects wrong-OS helpers instead of silently bundling them',()=>{
 assert.equal(nativeMagic(Buffer.from([0x7f,0x45,0x4c,0x46]),'linux'),true);
 assert.equal(nativeMagic(Buffer.from([0x7f,0x45,0x4c,0x46]),'win32'),false);
 assert.equal(nativeMagic(Buffer.from('MZ'),'win32'),true);
 assert.equal(nativeMagic(Buffer.from([0xcf,0xfa,0xed,0xfe]),'darwin'),true);
 assert.equal(nativeMagic(Buffer.from('bad'),'linux'),false);
});
