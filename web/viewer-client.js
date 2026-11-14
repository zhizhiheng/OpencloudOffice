// Asynchronous access to the viewer's single document. The synchronous SDK
// remains available in oco.js for other hosts.
export class ViewerClient {
  #worker;
  #pending = new Map();
  #next = 0;
  #failure = null;

  constructor({ onProgress = () => {}, onFailure = () => {} } = {}) {
    this.#worker = new Worker(new URL("./viewer-worker.js", import.meta.url), { type: "module" });
    const fail = (error) => {
      if (this.#failure) return;
      this.#failure = error;
      for (const { reject } of this.#pending.values()) reject(error);
      this.#pending.clear();
      this.#worker.terminate();
      onFailure(error);
    };
    this.#worker.addEventListener("error", (event) => {
      event.preventDefault();
      fail(new Error(event.message || "文档处理异常，请刷新页面后重试。"));
    });
    this.#worker.addEventListener("messageerror", () => fail(new Error("无法读取页面结果，请刷新页面后重试。")));
    this.#worker.addEventListener("message", ({ data }) => {
      if (data.type === "progress") return onProgress(data);
      if (data.fatal) return fail(new Error(`${data.error}。请刷新页面后重试。`));
      const pending = this.#pending.get(data.id);
      if (!pending) return;
      this.#pending.delete(data.id);
      if (data.error) pending.reject(new Error(data.error));
      else pending.resolve(data.result);
    });
  }

  request(type, args = {}, transfer = []) {
    if (this.#failure) return Promise.reject(this.#failure);
    return new Promise((resolve, reject) => {
      const id = ++this.#next;
      this.#pending.set(id, { resolve, reject });
      try {
        this.#worker.postMessage({ id, type, args }, transfer);
      } catch (error) {
        this.#pending.delete(id);
        reject(error);
      }
    });
  }

  open(bytes, name) {
    return this.request("open", { bytes, name }, [bytes]);
  }

  addFonts(buffers, revision) {
    return this.request("addFonts", { buffers, revision }, buffers);
  }
}
