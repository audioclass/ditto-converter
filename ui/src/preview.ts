// WebKitGTK cannot use the custom asset scheme directly as an audio source.
// Fetch only a backend-scoped generated file, bound memory, then use a Blob URL.
export async function previewBlob(src: string, fetcher: (url:string)=>Promise<Response> = fetch, limit = 128 * 1024 * 1024): Promise<Blob> {
  const response=await fetcher(src);
  if(!response.ok) throw new Error(`Preview could not be read (${response.status}).`);
  const size=Number(response.headers.get('Content-Length'));
  if(size>limit){await response.body?.cancel();throw new Error('Preview is too large for this desktop player (128 MiB limit). Conversion is still available.');}
  if(!response.body) throw new Error('Preview returned no audio.');
  const reader=response.body.getReader();const chunks:Uint8Array<ArrayBuffer>[]=[];let received=0;
  try {while(true){const {done,value}=await reader.read();if(done)break;received+=value.byteLength;if(received>limit)throw new Error('Preview is too large for this desktop player (128 MiB limit). Conversion is still available.');chunks.push(new Uint8Array(value));}}
  catch(error){await reader.cancel();throw error;}
  return new Blob(chunks,{type:response.headers.get('Content-Type')??'audio/wav'});
}
