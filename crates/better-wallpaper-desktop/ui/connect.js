const status = document.querySelector("#status");
const retry = document.querySelector("#retry");
const help = document.querySelector("#help");
async function connect() {
  retry.hidden = true;
  help.hidden = true;
  status.textContent = "正在连接壁纸服务…";
  try {
    await window.__TAURI__.core.invoke("connect");
  } catch (error) {
    status.textContent = `无法连接壁纸服务：${String(error)}`;
    help.hidden = false;
    retry.hidden = false;
  }
}
retry.addEventListener("click", connect);
connect();
