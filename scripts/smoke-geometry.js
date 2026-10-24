// Embedded only in macOS + smoke builds. Observations never decide acceptance.
export function createGeometryDiagnostics(invoke, readDOM, now) {
  const events = [];
  const record = (stage, request, helper) => {
    events.push({sequence:events.length+1, monotonicMs:now(), stage, request, dom:readDOM(), helper});
  };
  const snapshot = async (stage, request) => {
    try {
      const helper = await invoke('smoke_geometry');
      record(stage, request, helper);
    } catch (error) {
      record(stage, request, undefined);
      events[events.length-1].error = String(error);
      throw error;
    }
  };
  return {events, record, snapshot};
}
