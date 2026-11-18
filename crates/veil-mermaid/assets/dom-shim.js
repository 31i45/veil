// veil spike: minimal DOM shim for mermaid Tiny bundle inside QuickJS-ng.
// Pure JS, no imports: works identically in a Node vm context and in QuickJS globals.
// Host bridges (injected before eval):
//   __measureText(text, fontSize, fontWeight, fontFamily) -> {width, height}   (spike: hardcoded)
//   __logMissing(name)                                                     (collect missing API list)
// Pumps: __pump() runs due timers on a virtual clock (fast-forward), __pendingTimers() -> count.
(function (G) {
  "use strict";

  var missing = Object.create(null);
  function miss(name) {
    if (!missing[name]) {
      missing[name] = 1;
      if (typeof G.__logMissing === "function") G.__logMissing(name);
    }
  }
  G.__missingApis = missing;

  var XHTML = "http://www.w3.org/1999/xhtml";
  var SVG = "http://www.w3.org/2000/svg";
  var XLINK = "http://www.w3.org/1999/xlink";
  var NS_PREFIX = { xhtml: XHTML, svg: SVG, xlink: XLINK };

  function normTag(tag) {
    var i = tag.indexOf(":");
    if (i > 0 && NS_PREFIX[tag.slice(0, i)]) return tag.slice(i + 1);
    return tag;
  }
  function nsOf(tag, fallback) {
    var i = tag.indexOf(":");
    if (i > 0) {
      var p = NS_PREFIX[tag.slice(0, i)];
      if (p) return p;
    }
    return fallback || XHTML;
  }

  // ---------- console ----------
  var consoleBuf = [];
  var mkLog = function (level) {
    return function () {
      var parts = [];
      for (var i = 0; i < arguments.length; i++) {
        var a = arguments[i];
        try {
          parts.push(typeof a === "string" ? a : JSON.stringify(a));
        } catch (e) {
          parts.push(String(a));
        }
      }
      if (consoleBuf.length < 500) consoleBuf.push(level + ": " + parts.join(" "));
    };
  };
  G.console = {
    log: mkLog("log"),
    debug: mkLog("debug"),
    info: mkLog("info"),
    warn: mkLog("warn"),
    error: mkLog("error"),
    trace: mkLog("trace"),
    table: mkLog("table"),
    group: mkLog("group"),
    groupEnd: function () {},
    time: function () {},
    timeEnd: function () {},
    assert: function (c) { if (!c) mkLog("assert")("assertion failed"); },
    __buf: consoleBuf,
  };
  G.__consoleBuf = consoleBuf;

  // ---------- console ends; virtual clock + timers ----------
  var vtime = 0;
  var timers = [];
  var seq = 0;
  G.setTimeout = function (fn, ms) {
    var id = ++seq;
    timers.push({ id: id, fn: fn, due: vtime + (ms || 0), args: Array.prototype.slice.call(arguments, 2), interval: 0 });
    return id;
  };
  G.clearTimeout = G.clearInterval = function (id) {
    for (var i = 0; i < timers.length; i++) if (timers[i].id === id) { timers.splice(i, 1); return; }
  };
  G.setInterval = function (fn, ms) {
    var id = ++seq;
    timers.push({ id: id, fn: fn, due: vtime + (ms || 1), args: Array.prototype.slice.call(arguments, 2), interval: ms || 1 });
    return id;
  };
  G.requestAnimationFrame = function (cb) { return G.setTimeout(function () { cb(vtime); }, 0); };
  G.cancelAnimationFrame = G.clearTimeout;
  G.__pendingTimers = function () { return timers.length; };
  G.__pump = function () {
    var ran = 0;
    for (;;) {
      if (!timers.length) break;
      var best = -1;
      for (var i = 0; i < timers.length; i++) {
        if (best < 0 || timers[i].due < timers[best].due) best = i;
      }
      var t = timers.splice(best, 1)[0];
      vtime = Math.max(vtime, t.due) + 1;
      try { t.fn.apply(null, t.args); ran++; } catch (e) { G.__logMissing("timer-error:" + (e && e.message)); }
      if (ran > 10000) { G.__logMissing("timer-runaway"); break; }
    }
    return ran;
  };

  function now() { return vtime; }

  // ---------- text measurement (host bridge) ----------
  function measure(text, fontSize, fontWeight, fontFamily) {
    var r;
    if (G.__measureTextRaw) {
      var s = G.__measureTextRaw(String(text), Number(fontSize) || 16, Number(fontWeight) || 400, String(fontFamily || ""));
      var p = String(s).split("|");
      r = { width: parseFloat(p[0]) || 0, height: parseFloat(p[1]) || 0 };
    } else {
      r = G.__measureText(String(text), Number(fontSize) || 16, Number(fontWeight) || 400, String(fontFamily || ""));
      if (!r || typeof r.width !== "number") r = { width: 0, height: 0 };
    }
    return r;
  }
  function styleNum(style, name, fallback) {
    var v = style && style.getPropertyValue(name);
    if (!v) return fallback;
    var n = parseFloat(v);
    return isNaN(n) ? fallback : n;
  }

  // ---------- style ----------
  function StyleDecl(el) {
    this._el = el;
    this._props = {};
  }
  StyleDecl.prototype.setProperty = function (n, v) {
    if (v == null) { this.removeProperty(n); return; }
    this._props[n] = String(v);
  };
  StyleDecl.prototype.getPropertyValue = function (n) {
    var v = this._props[n];
    if (v != null) return v;
    var a = this._el && this._el.getAttribute && this._el.getAttribute("style");
    if (a && a.indexOf(n) >= 0) {
      var m = a.match(new RegExp("(?:^|;)\\s*" + n.replace(/[-]/g, "\\-") + "\\s*:\\s*([^;]+)", "i"));
      if (m) return m[1].trim();
    }
    return "";
  };
  StyleDecl.prototype.removeProperty = function (n) { delete this._props[n]; };
  Object.defineProperty(StyleDecl.prototype, "cssText", {
    get: function () {
      var out = "";
      for (var k in this._props) out += k + ":" + this._props[k] + ";";
      return out;
    },
    set: function (v) {
      this._props = {};
      var self = this;
      String(v || "").split(";").forEach(function (d) {
        var i = d.indexOf(":");
        if (i > 0) self.setProperty(d.slice(0, i).trim(), d.slice(i + 1).trim());
      });
    },
  });
  StyleDecl.prototype.length = 0;

  // ---------- classList ----------
  function ClassList(el) { this._el = el; }
  ClassList.prototype._set = function () {
    var s = {};
    String(this._el._attrs["class"] || "").split(/\s+/).forEach(function (c) { if (c) s[c] = 1; });
    return s;
  };
  ClassList.prototype._sync = function (s) { this._el.setAttribute("class", Object.keys(s).join(" ")); };
  ClassList.prototype.add = function () { var s = this._set(); for (var i = 0; i < arguments.length; i++) s[arguments[i]] = 1; this._sync(s); };
  ClassList.prototype.remove = function () { var s = this._set(); for (var i = 0; i < arguments.length; i++) delete s[arguments[i]]; this._sync(s); };
  ClassList.prototype.contains = function (c) { return !!this._set()[c]; };
  ClassList.prototype.toggle = function (c) { var s = this._set(); if (s[c]) { delete s[c]; } else { s[c] = 1; } this._sync(s); };
  Object.defineProperty(ClassList.prototype, "value", { get: function () { return this._el._attrs["class"] || ""; } });
  ClassList.prototype.toString = function () { return this.value; };

  // ---------- nodes ----------
  var NODE_TEXT = 3, NODE_RAW = 4;

  function TextNode(data) {
    this.nodeType = NODE_TEXT;
    this._data = String(data);
    this.parentNode = null;
    this.ownerDocument = null;
  }
  Object.defineProperty(TextNode.prototype, "data", {
    get: function () { return this._data; },
    set: function (v) { this._data = String(v); },
  });
  Object.defineProperty(TextNode.prototype, "textContent", {
    get: function () { return this._data; },
    set: function (v) { this._data = String(v); },
  });
  Object.defineProperty(TextNode.prototype, "nodeValue", {
    get: function () { return this._data; },
    set: function (v) { this._data = String(v); },
  });
  TextNode.prototype.cloneNode = function () { var n = new TextNode(this._data); n.ownerDocument = this.ownerDocument; return n; };
  TextNode.prototype.remove = function () { if (this.parentNode) this.parentNode.removeChild(this); };
  TextNode.prototype.contains = function () { return false; };
  TextNode.prototype.getBox = function () { return { x: 0, y: 0, width: 0, height: 0 }; };

  function RawNode(html) {
    this.nodeType = NODE_RAW;
    this._html = String(html);
    this.parentNode = null;
    this.ownerDocument = null;
    this.tagName = "#raw";
  }
  Object.defineProperty(RawNode.prototype, "textContent", {
    get: function () { return this._html; },
    set: function (v) { this._html = String(v); },
  });
  RawNode.prototype.cloneNode = function () { return new RawNode(this._html); };
  RawNode.prototype.remove = function () { if (this.parentNode) this.parentNode.removeChild(this); };
  RawNode.prototype.contains = function () { return false; };
  RawNode.prototype.getBox = function () { return { x: 0, y: 0, width: 0, height: 0 }; };

  function escapeAttr(v) {
    return String(v).replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;");
  }
  function escapeText(v) {
    return String(v).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  }

  function Element(tag, ns) {
    this.nodeType = 1;
    this.tagName = normTag(tag);
    this.namespaceURI = ns || nsOf(tag, XHTML);
    this._attrs = {};
    this.childNodes = [];
    this.parentNode = null;
    this.ownerDocument = null;
    this._style = new StyleDecl(this);
    this._classList = new ClassList(this);
  }
  Object.defineProperty(Element.prototype, "nodeName", { get: function () { return this.tagName; } });
  Object.defineProperty(Element.prototype, "id", {
    get: function () { return this._attrs["id"] || ""; },
    set: function (v) { this._attrs["id"] = String(v); },
  });
  Object.defineProperty(Element.prototype, "className", {
    get: function () { return this._attrs["class"] || ""; },
    set: function (v) { this._attrs["class"] = String(v); },
  });
  Object.defineProperty(Element.prototype, "style", { get: function () { return this._style; } });
  Object.defineProperty(Element.prototype, "classList", { get: function () { return this._classList; } });
  Object.defineProperty(Element.prototype, "children", {
    get: function () { return this.childNodes.filter(function (n) { return n.nodeType === 1; }); },
  });
  Object.defineProperty(Element.prototype, "innerHTML", {
    get: function () {
      var out = "";
      this.childNodes.forEach(function (n) {
        if (n.nodeType === NODE_TEXT) out += escapeText(n._data);
        else if (n.nodeType === NODE_RAW) out += n._html;
        else out += n.outerHTML;
      });
      return out;
    },
    set: function (v) {
      this.childNodes.length = 0;
      if (v != null && v !== "") this.childNodes.push(new RawNode(String(v)));
    },
  });
  Object.defineProperty(Element.prototype, "outerHTML", {
    get: function () {
      var tag = this.tagName;
      var attrs = "";
      for (var k in this._attrs) {
        attrs += " " + k + '="' + escapeAttr(this._attrs[k]) + '"';
      }
      if (this._style && this._style.cssText) attrs += ' style="' + escapeAttr(this._style.cssText) + '"';
      var inner = this.innerHTML;
      if (!inner) return "<" + tag + attrs + "></" + tag + ">";
      return "<" + tag + attrs + ">" + inner + "</" + tag + ">";
    },
  });
  Object.defineProperty(Element.prototype, "textContent", {
    get: function () {
      var out = "";
      this.childNodes.forEach(function (n) { out += n.textContent; });
      return out;
    },
    set: function (v) {
      this.childNodes.length = 0;
      if (v != null && v !== "") this.childNodes.push(new TextNode(String(v)));
    },
  });
  Object.defineProperty(Element.prototype, "firstChild", {
    get: function () { return this.childNodes[0] || null; },
  });
  Object.defineProperty(Element.prototype, "lastChild", {
    get: function () { return this.childNodes[this.childNodes.length - 1] || null; },
  });
  Object.defineProperty(Element.prototype, "nextSibling", {
    get: function () {
      var p = this.parentNode;
      if (!p) return null;
      var i = p.childNodes.indexOf(this);
      return p.childNodes[i + 1] || null;
    },
  });
  Object.defineProperty(Element.prototype, "previousSibling", {
    get: function () {
      var p = this.parentNode;
      if (!p) return null;
      var i = p.childNodes.indexOf(this);
      return i > 0 ? p.childNodes[i - 1] : null;
    },
  });
  Element.prototype.appendChild = function (n) {
    if (n === this) return n;
    if (n.parentNode) n.parentNode.removeChild(n);
    n.parentNode = this;
    if (!n.ownerDocument) n.ownerDocument = this.ownerDocument;
    this.childNodes.push(n);
    return n;
  };
  Element.prototype.insertBefore = function (n, ref) {
    if (n === this) return n;
    if (n.parentNode) n.parentNode.removeChild(n);
    if (!n.ownerDocument) n.ownerDocument = this.ownerDocument;
    if (ref == null) { return this.appendChild(n); }
    var i = this.childNodes.indexOf(ref);
    if (i < 0) return this.appendChild(n);
    n.parentNode = this;
    this.childNodes.splice(i, 0, n);
    return n;
  };
  Element.prototype.removeChild = function (n) {
    var i = this.childNodes.indexOf(n);
    if (i >= 0) {
      this.childNodes.splice(i, 1);
      n.parentNode = null;
    }
    return n;
  };
  Element.prototype.replaceChild = function (n, old) {
    var i = this.childNodes.indexOf(old);
    if (i >= 0) {
      this.removeChild(old);
      this.insertBefore(n, this.childNodes[i] || null);
    }
    return old;
  };
  Element.prototype.remove = function () { if (this.parentNode) this.parentNode.removeChild(this); };
  Element.prototype.hasChildNodes = function () { return this.childNodes.length > 0; };
  Element.prototype.cloneNode = function (deep) {
    var n = new Element(this.tagName, this.namespaceURI);
    for (var k in this._attrs) n._attrs[k] = this._attrs[k];
    n._style._props = {};
    for (var p in this._style._props) n._style._props[p] = this._style._props[p];
    if (deep) {
      this.childNodes.forEach(function (c) { n.appendChild(c.cloneNode(true)); });
    }
    return n;
  };
  Element.prototype.contains = function (n) {
    while (n) {
      if (n === this) return true;
      n = n.parentNode;
    }
    return false;
  };
  Element.prototype.closest = function (sel) {
    var el = this;
    while (el) {
      if (el.nodeType === 1 && matchesPartChain(el, sel)) return el;
      el = el.parentNode;
    }
    return null;
  };
  Element.prototype.setAttribute = function (n, v) { this._attrs[n] = String(v); };
  Element.prototype.getAttribute = function (n) {
    if (Object.prototype.hasOwnProperty.call(this._attrs, n)) return this._attrs[n];
    if (n === "style" && this._style && this._style.cssText) return this._style.cssText;
    return null;
  };
  Element.prototype.removeAttribute = function (n) { delete this._attrs[n]; };
  Element.prototype.hasAttribute = function (n) { return Object.prototype.hasOwnProperty.call(this._attrs, n); };
  Element.prototype.setAttributeNS = function (ns, n, v) { this._attrs[n] = String(v); };
  Element.prototype.getAttributeNS = function (ns, n) { return this.getAttribute(n); };
  Element.prototype.removeAttributeNS = function (ns, n) { delete this._attrs[n]; };
  Element.prototype.hasAttributeNS = function (ns, n) { return this.hasAttribute(n); };
  Element.prototype.addEventListener = function () { miss("addEventListener"); };
  Element.prototype.removeEventListener = function () { miss("removeEventListener"); };
  Element.prototype.dispatchEvent = function () { miss("dispatchEvent"); return true; };
  Element.prototype.focus = function () { miss("focus"); };
  Element.prototype.blur = function () { miss("blur"); };
  Element.prototype.scrollTo = function () { miss("scrollTo"); };
  Element.prototype.querySelectorAll = function (sel) { return queryAll(this, sel); };
  Element.prototype.querySelector = function (sel) { return queryAll(this, sel)[0] || null; };
  Element.prototype.getElementsByTagName = function (tag) {
    var out = [];
    walk(this, function (n) {
      if (n.nodeType === 1 && n.tagName.toLowerCase() === tag.toLowerCase()) out.push(n);
    });
    return out;
  };
  Element.prototype.getElementsByClassName = function (cls) {
    var out = [];
    walk(this, function (n) {
      if (n.nodeType === 1 && n._classList.contains(cls)) out.push(n);
    });
    return out;
  };
  Object.defineProperty(Element.prototype, "dataset", {
    get: function () {
      var self = this;
      var o = {};
      for (var k in this._attrs) {
        if (k.indexOf("data-") === 0) {
          var key = k.slice(5).replace(/-([a-z])/g, function (_m, c) { return c.toUpperCase(); });
          o[key] = this._attrs[k];
        }
      }
      o._owner = self;
      return o;
    },
  });

  // ---------- measurement ----------
  function parseTransform(s) {
    // returns {x, y, sx, sy} — supports translate/scale/matrix (e,f applied)
    var t = { x: 0, y: 0, sx: 1, sy: 1 };
    if (!s) return t;
    var re = /(translate|scale|matrix)\s*\(([^)]*)\)/g;
    var m;
    while ((m = re.exec(s))) {
      var args = m[2].split(/[\s,]+/).map(parseFloat).filter(function (n) { return !isNaN(n); });
      if (m[1] === "translate") {
        t.x += args[0] || 0;
        t.y += args[1] || 0;
      } else if (m[1] === "scale") {
        t.sx *= args[0] == null ? 1 : args[0];
        t.sy *= args[1] == null ? args[0] == null ? 1 : args[0] : args[1];
      } else if (m[1] === "matrix") {
        t.x += args[4] || 0;
        t.y += args[5] || 0;
        t.sx *= args[0] == null ? 1 : args[0];
        t.sy *= args[3] == null ? 1 : args[3];
      }
    }
    return t;
  }
  function union(a, b) {
    if (!b) return a;
    var x0 = Math.min(a.x, b.x), y0 = Math.min(a.y, b.y);
    var x1 = Math.max(a.x + a.width, b.x + b.width), y1 = Math.max(a.y + a.height, b.y + b.height);
    return { x: x0, y: y0, width: x1 - x0, height: y1 - y0 };
  }
  // 应用自身 transform（translate/scale/matrix），返回父坐标系中的盒。
  function boxApply(t, b) {
    return { x: b.x * t.sx + t.x, y: b.y * t.sy + t.y, width: b.width * t.sx, height: b.height * t.sy };
  }
  // 不渲染的子树不计入 getBBox（与浏览器一致：defs/marker/filter/渐变等）。
  var NON_RENDERED = { defs: 1, marker: 1, clipPath: 1, pattern: 1, filter: 1, style: 1, title: 1, desc: 1, linearGradient: 1, radialGradient: 1, symbol: 1 };
  function attrF(a, k, d) {
    var v = parseFloat(a[k]);
    return isNaN(v) ? d : v;
  }
  // 图形元素自身几何（浏览器 getBBox 语义：rect/circle/ellipse/line/polygon/path/image）。
  // 此前缺失导致 dagre 布局只看到 label 宽度（节点矩形互相重叠）、画布虚高。
  function shapeOwnBox(el) {
    var a = el._attrs || {};
    switch (el.tagName) {
      case "rect": {
        var w = attrF(a, "width", 0), h = attrF(a, "height", 0);
        if (w <= 0 || h <= 0) return null;
        return { x: attrF(a, "x", 0), y: attrF(a, "y", 0), width: w, height: h };
      }
      case "circle": {
        var r = attrF(a, "r", 0);
        if (r <= 0) return null;
        return { x: attrF(a, "cx", 0) - r, y: attrF(a, "cy", 0) - r, width: r * 2, height: r * 2 };
      }
      case "ellipse": {
        var rx = attrF(a, "rx", 0), ry = attrF(a, "ry", 0);
        if (rx <= 0 || ry <= 0) return null;
        return { x: attrF(a, "cx", 0) - rx, y: attrF(a, "cy", 0) - ry, width: rx * 2, height: ry * 2 };
      }
      case "line":
        return {
          x: Math.min(attrF(a, "x1", 0), attrF(a, "x2", 0)),
          y: Math.min(attrF(a, "y1", 0), attrF(a, "y2", 0)),
          width: Math.abs(attrF(a, "x1", 0) - attrF(a, "x2", 0)),
          height: Math.abs(attrF(a, "y1", 0) - attrF(a, "y2", 0)),
        };
      case "polygon":
      case "polyline": {
        var parts = String(a.points || "").trim().split(/[\s,]+/);
        var xs = [], ys = [];
        for (var i = 0; i + 1 < parts.length; i += 2) {
          var px = parseFloat(parts[i]), py = parseFloat(parts[i + 1]);
          if (!isNaN(px) && !isNaN(py)) { xs.push(px); ys.push(py); }
        }
        if (!xs.length) return null;
        return {
          x: Math.min.apply(null, xs), y: Math.min.apply(null, ys),
          width: Math.max.apply(null, xs) - Math.min.apply(null, xs),
          height: Math.max.apply(null, ys) - Math.min.apply(null, ys),
        };
      }
      case "path": {
        // 保守 bbox：取全部坐标对（含曲线控制点）。mermaid 生成的路径几乎全为绝对命令，
        // 相对命令（小写）仅影响画布尺寸精度，不影响节点布局。
        var nums = String(a.d || "").match(/-?\d*\.?\d+(?:e[-+]?\d+)?/gi);
        if (!nums || nums.length < 2) return null;
        var X = [], Y = [];
        for (var j = 0; j + 1 < nums.length; j += 2) { X.push(parseFloat(nums[j])); Y.push(parseFloat(nums[j + 1])); }
        return {
          x: Math.min.apply(null, X), y: Math.min.apply(null, Y),
          width: Math.max.apply(null, X) - Math.min.apply(null, X),
          height: Math.max.apply(null, Y) - Math.min.apply(null, Y),
        };
      }
      case "image":
        return { x: attrF(a, "x", 0), y: attrF(a, "y", 0), width: attrF(a, "width", 0), height: attrF(a, "height", 0) };
      default:
        return null;
    }
  }
  Element.prototype.getBox = function () {
    var tn = this.tagName;
    if (NON_RENDERED[tn]) return { x: 0, y: 0, width: 0, height: 0 };
    var t = parseTransform(this._attrs["transform"]);
    var own = shapeOwnBox(this);
    if (own) return boxApply(t, own);
    if (tn === "text") {
      var fs = styleNum(this._style, "font-size", parseFloat(this._attrs["font-size"]) || 16);
      var fw = styleNum(this._style, "font-weight", 400);
      var ff = this._style.getPropertyValue("font-family") || this._attrs["font-family"] || "sans-serif";
      var text = this.textContent || "";
      var lines = text.split(/\r?\n/);
      var w = 0, h = 0;
      lines.forEach(function (ln) {
        var r = measure(ln, fs, fw, ff);
        w = Math.max(w, r.width);
        h += r.height;
      });
      if (h === 0) { h = fs * 1.25; w = w || 0; }
      // x/y 属性 + text-anchor 定位（基线相对盒）；无属性时与浏览器一致按 0 基线处理。
      var xs = String(this._attrs["x"] || "").trim().split(/[\s,]+/);
      var ys = String(this._attrs["y"] || "").trim().split(/[\s,]+/);
      var x0 = xs.length && xs[0] !== "" && !isNaN(parseFloat(xs[0])) ? parseFloat(xs[0]) : 0;
      var y0 = ys.length && ys[0] !== "" && !isNaN(parseFloat(ys[0])) ? parseFloat(ys[0]) : 0;
      var anchor = this._style.getPropertyValue("text-anchor") || this._attrs["text-anchor"] || "";
      var bx = anchor === "middle" ? x0 - w / 2 : anchor === "end" ? x0 - w : x0;
      return boxApply(t, { x: bx, y: y0 - h, width: w, height: h });
    }
    // 容器：union 子元素。子元素 getBox 已应用其自身 transform（恰好一次），
    // 修复此前"子 transform 在子层与父层各应用一次"的双重叠加（画布虚高根因之一）。
    var box = null;
    this.childNodes.forEach(function (c) {
      if (!c.getBox) return;
      var cb = c.getBox();
      if (cb && (cb.width > 0 || cb.height > 0)) box = box ? union(box, cb) : cb;
    });
    if (!box) return { x: 0, y: 0, width: 0, height: 0 };
    return boxApply(t, box);
  };
  Element.prototype.getBBox = function () {
    miss("getBBox");
    return this.getBox();
  };
  // SVGTextElement metrics (used by mermaid's createFormattedText width wrapping)
  Element.prototype.textMetrics = function () {
    var fs = styleNum(this._style, "font-size", parseFloat(this._attrs["font-size"]) || 16);
    var fw = styleNum(this._style, "font-weight", 400);
    var ff = this._style.getPropertyValue("font-family") || this._attrs["font-family"] || "sans-serif";
    return { fs: fs, fw: fw, ff: ff };
  };
  Element.prototype.getComputedTextLength = function () {
    miss("getComputedTextLength");
    var m = this.textMetrics();
    return measure(this.textContent || "", m.fs, m.fw, m.ff).width;
  };
  Element.prototype.getSubStringLength = function (start, len) {
    miss("getSubStringLength");
    var m = this.textMetrics();
    return measure(String(this.textContent || "").slice(start, start + len), m.fs, m.fw, m.ff).width;
  };
  Element.prototype.getNumberOfChars = function () {
    return String(this.textContent || "").length;
  };
  Element.prototype.getBoundingClientRect = function () {
    miss("getBoundingClientRect");
    var b = this.getBox();
    return { x: b.x, y: b.y, width: b.width, height: b.height, left: b.x, top: b.y, right: b.x + b.width, bottom: b.y + b.height };
  };
  Element.prototype.getClientRects = function () {
    miss("getClientRects");
    var r = this.getBoundingClientRect();
    return [r];
  };
  Object.defineProperty(Element.prototype, "viewBox", {
    get: function () {
      var b = this.getBox();
      var v = String(this._attrs["viewBox"] || "0 0 0 0").split(/[\s,]+/).map(parseFloat);
      var base = { x: v[0] || 0, y: v[1] || 0, width: v[2] || 0, height: v[3] || 0 };
      return { baseVal: base, animVal: base };
    },
    set: function (v) { this._attrs["viewBox"] = String(v); },
  });
  Object.defineProperty(Element.prototype, "ownerSVGElement", {
    get: function () {
      var el = this.parentNode;
      while (el) {
        if (el.nodeType === 1 && el.tagName === "svg") return el;
        el = el.parentNode;
      }
      return null;
    },
  });

  // ---------- selectors ----------
  function walk(root, fn) {
    var kids = root.childNodes || [];
    for (var i = 0; i < kids.length; i++) {
      fn(kids[i]);
      walk(kids[i], fn);
    }
  }
  function parsePart(raw) {
    // pseudo-class 拆出来（d3 insert(el, ':first-child') 依赖它）
    var pseudo = null;
    var pm = raw.match(/:(first|last)-child$/);
    if (pm) {
      pseudo = pm[1] + "-child";
      raw = raw.slice(0, pm.index);
    }
    var m = raw.match(/^([a-zA-Z][\w-]*|\*)?((?:[.#][\w-]+|\[[^\]]+\])*)$/);
    if (!m) return null;
    var part = { tag: m[1] || null, id: null, classes: [], attrs: [], pseudo: pseudo };
    var rest = m[2] || "";
    var idm = rest.match(/#([\w-]+)/);
    if (idm) part.id = idm[1];
    var clsm = rest.match(/\.([\w-]+)/g);
    if (clsm) clsm.forEach(function (c) { part.classes.push(c.slice(1)); });
    var atm = rest.match(/\[([^\]]+)\]/g);
    if (atm) {
      atm.forEach(function (a) {
        var inner = a.slice(1, -1);
        var eq = inner.indexOf("=");
        if (eq >= 0) {
          var name = inner.slice(0, eq).trim();
          var val = inner.slice(eq + 1).trim().replace(/^["']|["']$/g, "");
          part.attrs.push({ name: name, value: val });
        } else {
          part.attrs.push({ name: inner.trim(), value: null });
        }
      });
    }
    return part;
  }
  function matchPart(el, part) {
    if (!el || el.nodeType !== 1) return false;
    if (part.pseudo === "first-child") {
      var p = el.parentNode;
      if (!p) return false;
      var first = null;
      for (var i = 0; i < p.childNodes.length; i++) {
        if (p.childNodes[i].nodeType === 1) { first = p.childNodes[i]; break; }
      }
      if (first !== el) return false;
    } else if (part.pseudo === "last-child") {
      var p2 = el.parentNode;
      if (!p2) return false;
      var last = null;
      for (var j = p2.childNodes.length - 1; j >= 0; j--) {
        if (p2.childNodes[j].nodeType === 1) { last = p2.childNodes[j]; break; }
      }
      if (last !== el) return false;
    }
    if (part.tag && part.tag !== "*" && el.tagName.toLowerCase() !== part.tag.toLowerCase()) return false;
    if (part.id && el.id !== part.id) return false;
    for (var i = 0; i < part.classes.length; i++) if (!el._classList.contains(part.classes[i])) return false;
    for (var j = 0; j < part.attrs.length; j++) {
      var a = part.attrs[j];
      var v = el.getAttribute(a.name);
      if (a.value == null) { if (v == null) return false; }
      else if (v !== a.value) return false;
    }
    return true;
  }
  function matchesPartChain(el, sel) {
    var sels = String(sel).split(",").map(function (s) { return s.trim(); }).filter(Boolean);
    return sels.some(function (one) {
      // tokenize into parts with combinators
      var toks = one.split(/\s*(>)\s*|\s+/).filter(Boolean);
      // rebuild parts respecting '>' capture groups
      var parts = [], comb = [], cur = "";
      // simpler re-tokenize
      parts = [];
      comb = [];
      var chunks = one.replace(/\s*>\s*/g, " > ").trim().split(/\s+/);
      var chain = [];
      for (var i = 0; i < chunks.length; i++) {
        if (chunks[i] === ">") { chain.push({ c: ">", p: parsePart(chunks[++i]) }); }
        else if (i === 0) { chain.push({ c: " ", p: parsePart(chunks[i]) }); }
        else { chain.push({ c: " ", p: parsePart(chunks[i]) }); }
      }
      if (!chain.length || chain.some(function (n) { return !n.p; })) return false;
      var node = el;
      if (!matchPart(node, chain[chain.length - 1].p)) return false;
      node = node.parentNode;
      for (var k = chain.length - 2; k >= 0; k--) {
        var want = chain[k], ok = false;
        if (want.c === ">") {
          if (node && matchPart(node, want.p)) { ok = true; node = node.parentNode; }
        } else {
          while (node) {
            if (matchPart(node, want.p)) { ok = true; node = node.parentNode; break; }
            node = node.parentNode;
          }
        }
        if (!ok) return false;
      }
      return true;
    });
  }
  function queryAll(root, sel) {
    var out = [];
    if (sel == null) return out;
    walk(root, function (n) {
      if (n.nodeType === 1 && matchesPartChain(n, sel)) out.push(n);
    });
    return out;
  }

  // ---------- document ----------
  var doc = new Element("#document", null);
  doc.nodeType = 9;
  doc.tagName = "#document";
  var docEl = new Element("html");
  docEl.ownerDocument = doc;
  var body = new Element("body");
  body.ownerDocument = doc;
  docEl.appendChild(body);
  doc.appendChild(docEl);
  doc.documentElement = docEl;
  doc.body = body;
  doc.head = new Element("head");
  docEl.insertBefore(doc.head, body);
  doc.createElement = function (tag) { var e = new Element(tag, XHTML); e.ownerDocument = doc; return e; };
  doc.createElementNS = function (ns, tag) {
    var real = ns === XHTML ? tag : tag;
    var e = new Element(real, ns || XHTML);
    e.ownerDocument = doc;
    return e;
  };
  doc.createTextNode = function (d) { var n = new TextNode(d); n.ownerDocument = doc; return n; };
  doc.createDocumentFragment = function () {
    var f = new Element("#document-fragment");
    f.nodeType = 11;
    f.ownerDocument = doc;
    return f;
  };
  doc.createComment = function (d) { return new RawNode("<!--" + d + "-->"); };
  doc.getElementById = function (id) {
    var found = null;
    walk(doc, function (n) {
      if (!found && n.nodeType === 1 && n.id === id) found = n;
    });
    return found;
  };
  doc.querySelector = function (sel) { return queryAll(doc, sel)[0] || null; };
  doc.querySelectorAll = function (sel) { return queryAll(doc, sel); };
  doc.addEventListener = function () { miss("document.addEventListener"); };
  doc.removeEventListener = function () {};
  doc.dispatchEvent = function () { return true; };
  doc.importNode = function (n) { return n.cloneNode(true); };
  doc.contains = function (n) { return doc === n || (n && n.ownerDocument === doc); };
  doc.hasFocus = function () { return true; };
  doc.title = "";
  doc.cookie = "";
  doc.characterSet = "UTF-8";
  doc.contentType = "text/html";
  doc.baseURI = "http://localhost/";
  doc.styleSheets = [];
  doc.createEvent = function () { return { initEvent: function () {}, preventDefault: function () {}, stopPropagation: function () {} }; };
  doc.createRange = function () { return { selectNode: function () {}, collapse: function () {}, setStart: function () {}, setEnd: function () {} }; };
  doc.implementation = {
    createHTMLDocument: function () {
      miss("document.implementation.createHTMLDocument");
      var d2 = new Element("#document", null);
      d2.nodeType = 9;
      var b2 = new Element("body");
      d2.appendChild(b2);
      d2.body = b2;
      return d2;
    },
  };
  doc.doctype = { name: "html" };
  doc.activeElement = body;

  // Note: doc is an Element (needs querySelector etc. — inherited via prototype already)

  // ---------- globals ----------
  var location = {
    href: "http://localhost/",
    protocol: "http:",
    host: "localhost",
    hostname: "localhost",
    port: "",
    origin: "http://localhost",
    pathname: "/",
    search: "",
    hash: "",
    toString: function () { return this.href; },
  };
  var storage = function () {
    var m = {};
    return {
      getItem: function (k) { return Object.prototype.hasOwnProperty.call(m, k) ? m[k] : null; },
      setItem: function (k, v) { m[k] = String(v); },
      removeItem: function (k) { delete m[k]; },
      clear: function () { m = {}; },
      key: function (i) { return Object.keys(m)[i] || null; },
      get length() { return Object.keys(m).length; },
    };
  };

  G.document = doc;
  G.location = location;
  G.navigator = {
    userAgent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36",
    platform: "Win32",
    language: "zh-CN",
    languages: ["zh-CN", "en-US"],
    onLine: true,
  };
  G.localStorage = storage();
  G.sessionStorage = storage();
  G.screen = { width: 1920, height: 1080, availWidth: 1920, availHeight: 1040 };
  G.devicePixelRatio = 1;
  G.addEventListener = function () { miss("window.addEventListener"); };
  G.removeEventListener = function () {};
  G.dispatchEvent = function () { return true; };
  G.alert = function () { miss("alert"); };
  G.confirm = function () { miss("confirm"); return false; };
  G.prompt = function () { miss("prompt"); return null; };
  G.open = function () { miss("window.open"); return null; };
  G.scrollTo = function () {};
  G.getComputedStyle = function (el) {
    miss("getComputedStyle");
    if (!el) return new StyleDecl(null);
    var s = new StyleDecl(el);
    s._props = {};
    return s;
  };
  G.matchMedia = function () {
    miss("matchMedia");
    return { matches: false, media: "", addEventListener: function () {}, removeEventListener: function () {}, addListener: function () {}, removeListener: function () {} };
  };
  G.history = { pushState: function () {}, replaceState: function () {}, back: function () {}, forward: function () {}, go: function () {} };
  G.self = G;
  G.top = G;
  G.parent = G;
  G.frames = G;
  G.btoa = function (s) {
    var chars = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    var bytes = [];
    for (var i = 0; i < s.length; i++) {
      var c = s.charCodeAt(i);
      if (c > 255) bytes.push(0xEF, 0xBF, 0xBD);
      else bytes.push(c);
    }
    var out = "";
    for (var j = 0; j < bytes.length; j += 3) {
      var b0 = bytes[j], b1 = bytes[j + 1], b2 = bytes[j + 2];
      out += chars[b0 >> 2];
      out += chars[((b0 & 3) << 4) | ((b1 == null ? 0 : b1) >> 4)];
      out += b1 == null ? "=" : chars[((b1 & 15) << 2) | ((b2 == null ? 0 : b2) >> 6)];
      out += b2 == null ? "=" : chars[b2 & 63];
    }
    return out;
  };
  G.atob = function (s) {
    var chars = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    s = String(s).replace(/=+$/, "");
    var out = "";
    var buf = 0, bits = 0;
    for (var i = 0; i < s.length; i++) {
      var v = chars.indexOf(s[i]);
      if (v < 0) continue;
      buf = (buf << 6) | v;
      bits += 6;
      if (bits >= 8) {
        bits -= 8;
        out += String.fromCharCode((buf >> bits) & 0xff);
      }
    }
    return out;
  };
  function TextEncoderShim() {}
  TextEncoderShim.prototype.encode = function (s) {
    var utf8 = unescape(encodeURIComponent(String(s)));
    var out = new Uint8Array(utf8.length);
    for (var i = 0; i < utf8.length; i++) out[i] = utf8.charCodeAt(i) & 0xff;
    return out;
  };
  G.TextEncoder = TextEncoderShim;
  function TextDecoderShim() {}
  TextDecoderShim.prototype.decode = function (bytes) {
    var s = "";
    for (var i = 0; i < bytes.length; i++) s += String.fromCharCode(bytes[i]);
    try {
      return decodeURIComponent(escape(s));
    } catch (e) {
      return s;
    }
  };
  G.TextDecoder = TextDecoderShim;
  G.crypto = {
    randomUUID: function () {
      return "veil-uuid-" + ++seq + "-" + Math.floor(Math.random() * 1e9);
    },
    getRandomValues: function (arr) {
      for (var i = 0; i < arr.length; i++) arr[i] = Math.floor(Math.random() * 256);
      return arr;
    },
  };
  G.performance = {
    now: function () { return vtime; },
    timeOrigin: 0,
    mark: function () {},
    measure: function () {},
    getEntriesByType: function () { return []; },
  };
  function CSSStyleSheetShim() {
    this.cssRules = [];
    this.ownerNode = null;
  }
  CSSStyleSheetShim.prototype.insertRule = function (rule, index) {
    var i = index == null ? this.cssRules.length : index;
    this.cssRules.splice(i, 0, { cssText: String(rule) });
    return i;
  };
  CSSStyleSheetShim.prototype.deleteRule = function (i) { this.cssRules.splice(i, 1); };
  CSSStyleSheetShim.prototype.replaceSync = function (css) {
    miss("CSSStyleSheet.replaceSync");
    this.cssRules = [{ cssText: String(css) }];
  };
  G.CSSStyleSheet = CSSStyleSheetShim;
  G.DOMParser = function () {
    miss("DOMParser");
    this.parseFromString = function (s) {
      var d = new Element("#document", null);
      d.nodeType = 9;
      var b = new Element("body");
      d.appendChild(b);
      d.body = b;
      d.documentElement = b;
      b.innerHTML = s;
      return d;
    };
  };
  G.XMLSerializer = function () {
    this.serializeToString = function (n) { return n.outerHTML != null ? n.outerHTML : n.textContent || ""; };
  };
  G.MutationObserver = function () {
    this.observe = function () { miss("MutationObserver.observe"); };
    this.disconnect = function () {};
    this.takeRecords = function () { return []; };
  };
  G.Node = { ELEMENT_NODE: 1, TEXT_NODE: 3, COMMENT_NODE: 8, DOCUMENT_NODE: 9, DOCUMENT_FRAGMENT_NODE: 11 };
  G.Image = function () { miss("Image"); };
  G.DOMTokenList = ClassList;
  G.HTMLElement = Element;
  G.SVGElement = Element;
  G.NodeFilter = { SHOW_ALL: 4294967295 };
  G.structuredClone = function (v) {
    var seen = new Map();
    function cl(x) {
      if (x == null || typeof x !== "object") return x;
      if (x instanceof Date) return new Date(x.getTime());
      if (seen.has(x)) return seen.get(x);
      if (Array.isArray(x)) {
        var a = [];
        seen.set(x, a);
        x.forEach(function (item, i) { a[i] = cl(item); });
        return a;
      }
      if (x instanceof Map) {
        var m = new Map();
        seen.set(x, m);
        x.forEach(function (val, k) { m.set(k, cl(val)); });
        return m;
      }
      if (x instanceof Set) {
        var s = new Set();
        seen.set(x, s);
        x.forEach(function (item) { s.add(cl(item)); });
        return s;
      }
      var o = {};
      seen.set(x, o);
      for (var k in x) if (Object.prototype.hasOwnProperty.call(x, k)) o[k] = cl(x[k]);
      return o;
    }
    return cl(v);
  };
  G.CustomEvent = function (t) { this.type = t; };
  G.Event = function (t) { this.type = t; };

  // window-level DOM aliases
  G.Element = Element;
  G.window = G;
  G.globalThis = G;
})(globalThis);
