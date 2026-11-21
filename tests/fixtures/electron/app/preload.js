// The page reaches the main process through a context bridge, as
// Electron apps do (contextIsolation is on by default).
const { contextBridge, ipcRenderer } = require('electron');
contextBridge.exposeInMainWorld('doors', {
  ping: (v) => ipcRenderer.invoke('ping', v),
});
