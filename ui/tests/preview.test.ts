import { test } from 'node:test';
import assert from 'node:assert/strict';
import { previewBlob } from '../src/preview.ts';
test('scoped preview fetch preserves WAV type and bytes', async () => {
 const blob=await previewBlob('asset://localhost/test.wav',async () =>new Response(new Uint8Array([82,73,70,70]),{headers:{'Content-Type':'audio/x-wav','Content-Length':'4'}}),8);
 assert.equal(blob.size,4);assert.equal(blob.type,'audio/x-wav');
});
test('preview memory bound rejects oversized and failed responses',async()=>{
 await assert.rejects(previewBlob('asset://localhost/a',async()=>new Response(new Uint8Array(9)),8),/too large/);
 await assert.rejects(previewBlob('asset://localhost/a',async()=>new Response('denied',{status:403})),/403/);
});
