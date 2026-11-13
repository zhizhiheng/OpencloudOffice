// JavaScript bindings for oco.wasm, written by hand: no generated glue and
// nothing to install. Works in browsers and in Node.
//
//   const oco = await Oco.load("oco.wasm");
//   oco.addFont(await (await fetch("fonts/SourceHanSerifCN-Regular.otf")).arrayBuffer());
//   const doc = oco.open(await file.arrayBuffer(), file.name);   // .docx, .doc or Markdown
//   for (let i = 0; i < doc.pageCount; i++) container.innerHTML += doc.pageSvg(i);
//   doc.close();

const text = new TextDecoder();

/** The core, loaded. One instance can hold any number of open documents. */
export class Oco {
  #wasm;
  #broken = null;

  /**
   * Loads the module from a URL, a `Response`, or its bytes.
   * @param {string | URL | Response | BufferSource} source
   */
  static async load(source) {
    let instance;
    if (source instanceof ArrayBuffer || ArrayBuffer.isView(source)) {
      ({ instance } = await WebAssembly.instantiate(source, {}));
    } else {
      const response = source instanceof Response ? source : await fetch(source);
      if (!response.ok) throw new Error(`${response.url}: HTTP ${response.status}`);
      // Streaming needs the server to say `application/wasm`; fall back when it does not.
      const copy = response.clone();
      try {
        ({ instance } = await WebAssembly.instantiateStreaming(response, {}));
      } catch {
        ({ instance } = await WebAssembly.instantiate(await copy.arrayBuffer(), {}));
      }
    }
    return new Oco(instance.exports);
  }

  constructor(exports) {
    this.#wasm = exports;
  }

  /**
   * Adds a font file (.ttf, .otf or .ttc) to the fonts documents are set in.
   * Open documents are laid out afresh the next time they are asked for a page.
   * @param {BufferSource} bytes
   * @returns {number} how many faces the file held; 0 if it is not a font
   */
  addFont(bytes) {
    // The core keeps the buffer, so it is not freed here.
    const [ptr, length] = this.#give(bytes);
    return this.#call(() => this.#wasm.oco_add_font(ptr, length));
  }

  /**
   * Opens a document: a Word document (.docx, or the older .doc) or Markdown.
   * What it is is told from the bytes; the file's name, when given, settles
   * what they leave open (text named .docx is a damaged Word document, not Markdown).
   * @param {BufferSource} bytes
   * @param {string} [name]
   * @returns {OcoDocument}
   * @throws {Error} with the reason when the bytes are not a readable document
   */
  open(bytes, name) {
    const [ptr, length] = this.#give(bytes);
    let handle;
    if (name) {
      const [namePtr, nameLength] = this.#give(new TextEncoder().encode(name));
      handle = this.#call(() => this.#wasm.oco_open_named(ptr, length, namePtr, nameLength));
      this.#wasm.oco_free(namePtr, nameLength);
    } else {
      handle = this.#call(() => this.#wasm.oco_open(ptr, length));
    }
    this.#wasm.oco_free(ptr, length);
    if (handle === 0) throw new Error(this.#result());
    return new OcoDocument(this.#api(), handle);
  }

  #give(bytes) {
    const view = bytes instanceof ArrayBuffer ? new Uint8Array(bytes) : new Uint8Array(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    const ptr = this.#call(() => this.#wasm.oco_alloc(view.length));
    // Taken after the allocation: growing the memory replaces its buffer.
    new Uint8Array(this.#wasm.memory.buffer, ptr, view.length).set(view);
    return [ptr, view.length];
  }

  #result() {
    const ptr = this.#wasm.oco_result_ptr();
    return text.decode(new Uint8Array(this.#wasm.memory.buffer, ptr, this.#wasm.oco_result_len()));
  }

  /** Runs a call into the module. A trap there is the core giving up: say why, and refuse further calls. */
  #call(run) {
    if (this.#broken) throw this.#broken;
    try {
      return run();
    } catch (error) {
      if (!(error instanceof WebAssembly.RuntimeError)) throw error;
      let why = error.message;
      try {
        const length = this.#wasm.oco_panic_len();
        if (length > 0) why = text.decode(new Uint8Array(this.#wasm.memory.buffer, this.#wasm.oco_panic_ptr(), length));
      } catch {}
      this.#broken = new Error(`the document core stopped: ${why}`, { cause: error });
      throw this.#broken;
    }
  }

  #api() {
    return { wasm: this.#wasm, call: (run) => this.#call(run), result: () => this.#result() };
  }
}

/** An open document. Pages are counted from 0 and measured in points (1/72 inch). */
export class OcoDocument {
  #core;
  #handle;

  constructor(core, handle) {
    this.#core = core;
    this.#handle = handle;
  }

  /** @returns {number} */
  get pageCount() {
    return this.#core.call(() => this.#core.wasm.oco_page_count(this.#handle));
  }

  /** @returns {{width: number, height: number}} */
  pageSize(page) {
    const { wasm, call } = this.#core;
    return call(() => ({ width: wasm.oco_page_width(this.#handle, page), height: wasm.oco_page_height(this.#handle, page) }));
  }

  /**
   * The page drawn as an SVG document. Text is drawn as outlines, so it looks
   * the same wherever it is shown and needs no fonts installed there.
   * @returns {string}
   */
  pageSvg(page) {
    const drawn = this.#core.call(() => this.#core.wasm.oco_page_svg(this.#handle, page));
    if (!drawn) throw new RangeError(`no page ${page}`);
    return this.#core.result();
  }

  /**
   * What the document is and what in it is not shown as Word shows it.
   * @returns {{pages: number, missingFonts: string[], notShown: {kind: string, count: number}[], warnings: string[]}}
   */
  info() {
    this.#core.call(() => this.#core.wasm.oco_info(this.#handle));
    return JSON.parse(this.#core.result());
  }

  /** Frees the document. It must not be used afterwards. */
  close() {
    this.#core.call(() => this.#core.wasm.oco_close(this.#handle));
    this.#handle = 0;
  }
}
