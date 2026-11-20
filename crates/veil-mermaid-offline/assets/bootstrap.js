// headless DOM 环境引导：linkedom 提供真实 DOM 树 + HTML/CSS 解析，
// 这里补齐 mermaid/d3/DOMPurify 需要的全局对象与 SVG 文本度量。
(function () {
  var L = globalThis.__linkedom;
  if (!L) throw new Error('linkedom missing');

  var doc = L.parseHTML('<!DOCTYPE html><html><head></head><body></body></html>').document;

  // 构造器挂到全局：mermaid 内有 `instanceof HTMLElement` 之类的分支，
  // DOMPurify 需要 HTMLFormElement / NamedNodeMap 等做节点合法性判断。
  var names = Object.getOwnPropertyNames(L);
  for (var i = 0; i < names.length; i++) {
    var n = names[i];
    if (/^[A-Z]/.test(n) && typeof L[n] === 'function' && !globalThis[n]) {
      globalThis[n] = L[n];
    }
  }
  if (!globalThis.NamedNodeMap) {
    var attrs = doc.createElement('div').attributes;
    if (attrs) globalThis.NamedNodeMap = Object.getPrototypeOf(attrs).constructor;
  }

  if (!doc.implementation) {
    doc.implementation = {
      createHTMLDocument: function () {
        return L.parseHTML('<!DOCTYPE html><html><head></head><body></body></html>').document;
      },
      createDocument: function () {
        return L.parseHTML('<!DOCTYPE html><html><head></head><body></body></html>').document;
      },
      hasFeature: function () { return true; }
    };
  }

  // linkedom 的 parseFromString 对 HTML 片段不补 <html>/<body>（body 为空），
  // 而 DOMPurify 依赖 body 承载片段。这里包一层：片段一律放进完整文档骨架。
  var NativeParser = L.DOMParser;
  if (NativeParser) {
    var WrappedParser = function () {};
    WrappedParser.prototype.parseFromString = function (src, type) {
      var text = String(src == null ? '' : src);
      if (/<html[\s>]/i.test(text) || /<body[\s>]/i.test(text)) {
        return new NativeParser().parseFromString(text, type);
      }
      return L.parseHTML('<!DOCTYPE html><html><head></head><body>' + text + '</body></html>').document;
    };
    globalThis.DOMParser = WrappedParser;
  }
  if (!doc.defaultView) {
    try { doc.defaultView = globalThis; } catch (e) {}
  }

  globalThis.document = doc;
  globalThis.window = globalThis;
  globalThis.self = globalThis;

  // rough.js 的 SVG path parser 仍读取旧式 RegExp.$1；QuickJS 不会在
  // String.prototype.match 后更新这些静态捕获值，需显式桥接。
  var nativeStringMatch = String.prototype.match;
  String.prototype.match = function (regexp) {
    var matched = nativeStringMatch.call(this, regexp);
    if (matched) {
      for (var capture = 1; capture <= 9; capture++) {
        try { RegExp['$' + capture] = matched[capture]; } catch (e) {}
      }
    }
    return matched;
  };

  // ---- 浏览器 API 兜底（QuickJS 无宿主）----
  if (!globalThis.console) {
    globalThis.console = {
      log: function () {}, warn: function () {}, error: function () {}, info: function () {},
      debug: function () {}, trace: function () {}, group: function () {}, groupEnd: function () {},
      time: function () {}, timeEnd: function () {}, assert: function () {}
    };
  }
  if (!globalThis.addEventListener) globalThis.addEventListener = function () {};
  if (!globalThis.removeEventListener) globalThis.removeEventListener = function () {};
  if (!globalThis.dispatchEvent) globalThis.dispatchEvent = function () { return true; };
  if (!globalThis.requestAnimationFrame) globalThis.requestAnimationFrame = function () { return 0; };
  if (!globalThis.cancelAnimationFrame) globalThis.cancelAnimationFrame = function () {};
  if (!globalThis.queueMicrotask) {
    globalThis.queueMicrotask = function (fn) { Promise.resolve().then(fn); };
  }
  if (!globalThis.matchMedia) {
    globalThis.matchMedia = function () {
      return { matches: false, addListener: function () {}, removeListener: function () {},
               addEventListener: function () {}, removeEventListener: function () {} };
    };
  }
  if (!globalThis.navigator) {
    globalThis.navigator = { userAgent: 'veil', platform: 'win32', language: 'zh-CN' };
  }
  if (!globalThis.location) {
    globalThis.location = { href: 'about:blank', protocol: 'about:', origin: 'null', search: '' };
  }
  if (!globalThis.screen) {
    globalThis.screen = { width: 1920, height: 1080, availWidth: 1920, availHeight: 1080 };
  }
  // 定时器只入队：渲染全程是微任务，宿主在主循环空转时调用 __veilFlushTimers 推进。
  if (!globalThis.setTimeout) {
    globalThis.setTimeout = function (fn) {
      (globalThis.__veilTimers = globalThis.__veilTimers || []).push(fn);
      return 0;
    };
  }
  if (!globalThis.clearTimeout) globalThis.clearTimeout = function () {};
  if (!globalThis.__veilFlushTimers) {
    globalThis.__veilFlushTimers = function () {
      var q = globalThis.__veilTimers || [];
      globalThis.__veilTimers = [];
      for (var i = 0; i < q.length; i++) {
        try { if (typeof q[i] === 'function') q[i](); } catch (e) {}
      }
      return q.length;
    };
  }

  if (!globalThis.getComputedStyle) {
    globalThis.getComputedStyle = function (el) {
      var st = (el && el.style) || {};
      var view = {
        fontSize: st.fontSize || '16px',
        fontFamily: st.fontFamily || 'sans-serif',
        fontWeight: st.fontWeight || '400',
        lineHeight: st.lineHeight || 'normal',
        display: st.display || 'block',
        color: st.color || '#000',
        getPropertyValue: function (name) {
          var key = String(name).replace(/-([a-z])/g, function (m, c) { return c.toUpperCase(); });
          var v = st[key];
          if (v !== undefined && v !== '') return String(v);
          if (key === 'fontSize') return view.fontSize;
          if (key === 'fontFamily') return view.fontFamily;
          if (key === 'fontWeight') return view.fontWeight;
          if (key === 'lineHeight') return view.lineHeight;
          // cytoscape 用这些做尺寸计算，空字符串会让 parseFloat 返回 NaN
          if (/^(padding|margin|border|gap)(Top|Right|Bottom|Left|$)/.test(key)) return '0px';
          if (/^(width|height|minWidth|minHeight|maxWidth|maxHeight)/.test(key)) return '0px';
          return '';
        }
      };
      return view;
    };
  }

  // ---- SVG 文本度量 ----
  // 宽度优先走宿主注入的真实度量（Rust 侧复用字体排版栈）；否则按字符估算。
  function measure(text, fontSize, family) {
    if (typeof globalThis.__veilMeasureText === 'function') {
      var w = globalThis.__veilMeasureText(String(text), String(family || ''), Number(fontSize) || 16);
      if (typeof w === 'number' && isFinite(w) && w >= 0) return w;
    }
    return String(text).length * (Number(fontSize) || 16) * 0.6;
  }

  function fontSizeOf(el) {
    var fs = 16;
    try {
      var attr = el.getAttribute && el.getAttribute('font-size');
      if (attr) fs = parseFloat(attr) || fs;
      else if (el.style && el.style.fontSize) fs = parseFloat(el.style.fontSize) || fs;
    } catch (e) {}
    return fs;
  }

  function ownText(el) {
    // 仅取直接文本子节点（d3 的 text 元素通常如此）
    var s = '';
    var kids = (el.childNodes || []);
    for (var i = 0; i < kids.length; i++) {
      var k = kids[i];
      if (k && (k.nodeType === 3 || k.nodeType === 4)) s += k.textContent || '';
    }
    return s || '';
  }

  var proto = L.Element.prototype;
  function tagOf(el) { return String(el.tagName || el.nodeName || '').toLowerCase(); }
  function attrNum(el, name) {
    var v = el.getAttribute && el.getAttribute(name);
    var n = parseFloat(v);
    return isNaN(n) ? 0 : n;
  }

  // 解析 transform 属性，返回 {a,b,c,d,e,f} 仿射矩阵
  function parseTransform(el) {
    if (!el || !el.getAttribute) return null;
    var t = el.getAttribute('transform');
    if (!t) return null;
    var m = { a: 1, b: 0, c: 0, d: 1, e: 0, f: 0 }; // identity
    // matrix(a,b,c,d,e,f)
    var mm = t.match(/matrix\(\s*([-\d.e]+)\s+([-\d.e]+)\s+([-\d.e]+)\s+([-\d.e]+)\s+([-\d.e]+)\s+([-\d.e]+)\s*\)/);
    if (mm) { m.a = parseFloat(mm[1]); m.b = parseFloat(mm[2]); m.c = parseFloat(mm[3]); m.d = parseFloat(mm[4]); m.e = parseFloat(mm[5]); m.f = parseFloat(mm[6]); return m; }
    // translate(x[, y])
    var mt = t.match(/translate\(\s*([-\d.e]+)\s*(?:,\s*([-\d.e]+)\s*)?\)/);
    if (mt) { m.e = parseFloat(mt[1]); m.f = mt[2] !== undefined ? parseFloat(mt[2]) : 0; return m; }
    // scale(x[, y])
    var ms = t.match(/scale\(\s*([-\d.e]+)\s*(?:,\s*([-\d.e]+)\s*)?\)/);
    if (ms) { m.a = parseFloat(ms[1]); m.d = ms[2] !== undefined ? parseFloat(ms[2]) : m.a; return m; }
    // rotate(angle)
    var mr = t.match(/rotate\(\s*([-\d.e]+)\s*\)/);
    if (mr) {
      var ang = parseFloat(mr[1]) * Math.PI / 180;
      m.a = Math.cos(ang); m.b = Math.sin(ang); m.c = -Math.sin(ang); m.d = Math.cos(ang);
      return m;
    }
    return m; // fallback: identity
  }

  // 对 bbox 应用仿射变换
  function transformBBox(bbox, m) {
    if (!m || (m.a === 1 && m.b === 0 && m.c === 0 && m.d === 1 && m.e === 0 && m.f === 0)) return bbox;
    // bbox 的四个角
    var x0 = bbox.x, y0 = bbox.y;
    var x1 = bbox.x + bbox.width, y1 = bbox.y;
    var x2 = bbox.x, y2 = bbox.y + bbox.height;
    var x3 = bbox.x + bbox.width, y3 = bbox.y + bbox.height;
    // 变换
    function tpx(x, y) { return { x: m.a * x + m.c * y + m.e, y: m.b * x + m.d * y + m.f }; }
    var p0 = tpx(x0, y0), p1 = tpx(x1, y1), p2 = tpx(x2, y2), p3 = tpx(x3, y3);
    var minX = Math.min(p0.x, p1.x, p2.x, p3.x);
    var minY = Math.min(p0.y, p1.y, p2.y, p3.y);
    var maxX = Math.max(p0.x, p1.x, p2.x, p3.x);
    var maxY = Math.max(p0.y, p1.y, p2.y, p3.y);
    return { x: minX, y: minY, width: maxX - minX, height: maxY - minY };
  }

  // 累积元素的完整 transform 链（从文档根到当前元素）
  function cumulativeTransform(el) {
    var m = { a: 1, b: 0, c: 0, d: 1, e: 0, f: 0 };
    var node = el;
    while (node && node.nodeType === 1) {
      var tm = parseTransform(node);
      if (tm) {
        // 左乘：M_total = M_parent * M_current
        var newM = {
          a: m.a * tm.a + m.c * tm.b,
          b: m.b * tm.a + m.d * tm.b,
          c: m.a * tm.c + m.c * tm.d,
          d: m.b * tm.c + m.d * tm.d,
          e: m.a * tm.e + m.c * tm.f + m.e,
          f: m.b * tm.e + m.d * tm.f + m.f
        };
        m = newM;
      }
      node = node.parentNode;
    }
    return m;
  }

  // path 的数值不是独立 (x,y) 序列：h/v 只有一个参数，曲线和弧线也有
  // 不同元数。按命令推进当前点，避免把相对位移误当成绝对坐标。
  function pathBBox(d) {
    var tokens = String(d || '').match(/[a-zA-Z]|[-+]?(?:\d*\.\d+|\d+\.?)(?:[eE][-+]?\d+)?/g) || [];
    var index = 0, command = null;
    var x = 0, y = 0, startX = 0, startY = 0;
    var minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
    function number() { return parseFloat(tokens[index++]); }
    function point(px, py) {
      if (!isFinite(px) || !isFinite(py)) return;
      minX = Math.min(minX, px); minY = Math.min(minY, py);
      maxX = Math.max(maxX, px); maxY = Math.max(maxY, py);
    }
    function pair(relative) {
      var px = number(), py = number();
      if (relative) { px += x; py += y; }
      point(px, py);
      return { x: px, y: py };
    }
    while (index < tokens.length) {
      if (/^[a-zA-Z]$/.test(tokens[index])) command = tokens[index++];
      if (!command) break;
      var upper = command.toUpperCase(), relative = command !== upper;
      if (upper === 'Z') {
        x = startX; y = startY; point(x, y); command = null;
        continue;
      }
      var count = ({ M: 2, L: 2, T: 2, H: 1, V: 1, C: 6, S: 4, Q: 4, A: 7 })[upper];
      if (!count || index + count > tokens.length || /^[a-zA-Z]$/.test(tokens[index])) {
        command = null;
        continue;
      }
      var end;
      if (upper === 'M' || upper === 'L' || upper === 'T') {
        end = pair(relative); x = end.x; y = end.y;
        if (upper === 'M') { startX = x; startY = y; }
      } else if (upper === 'H') {
        var hx = number(); x = relative ? x + hx : hx; point(x, y);
      } else if (upper === 'V') {
        var vy = number(); y = relative ? y + vy : vy; point(x, y);
      } else if (upper === 'C') {
        pair(relative); pair(relative); end = pair(relative); x = end.x; y = end.y;
      } else if (upper === 'S' || upper === 'Q') {
        pair(relative); end = pair(relative); x = end.x; y = end.y;
      } else if (upper === 'A') {
        index += 5; end = pair(relative); x = end.x; y = end.y;
      }
      if (upper === 'M') command = relative ? 'l' : 'L';
    }
    if (!isFinite(minX) || maxX < minX || maxY < minY) return null;
    return { x: minX, y: minY, width: maxX - minX, height: maxY - minY };
  }

  proto.getBBox = function () {
    var fs = fontSizeOf(this);
    var tag = tagOf(this);
    // style/defs/clipPath 等不参与可见边界；display:none 的子树同样排除。
    // 否则根 SVG 会把数千字符的 CSS 当成一行文本，或把隐藏布局模板计入尺寸。
    if (/^(style|script|defs|marker|clippath|mask|pattern|symbol|metadata|title|desc)$/.test(tag) ||
        (this.style && this.style.display === 'none') ||
        (this.getAttribute && this.getAttribute('display') === 'none')) {
      return { x: 0, y: 0, width: 0, height: 0 };
    }
    var isText = /^(text|tspan|textpath)$/.test(tag);
    // 只有真正的 SVG 文本节点才能用 textContent 测量。容器（尤其根 SVG）
    // 的 textContent 还包含 CSS 与所有后代标签，必须改为合并子元素 bbox。
    var text = isText ? (this.textContent || '').trim() : '';

    var result;

    if (!isText) {
      // 几何元素：直接用 width/height/圆心等属性。根 svg 的 100% 不是布局尺寸，
      // 因此 svg 本身走下面的子元素合并路径。
      if (tag === 'rect' || tag === 'image' || tag === 'foreignobject') {
        var w0 = attrNum(this, 'width'), h0 = attrNum(this, 'height');
        if (w0 > 0 || h0 > 0) {
          result = { x: attrNum(this, 'x'), y: attrNum(this, 'y'), width: w0, height: h0 };
        }
      } else if (tag === 'circle') {
        var r = attrNum(this, 'r');
        if (r > 0) {
          result = { x: attrNum(this, 'cx') - r, y: attrNum(this, 'cy') - r, width: 2 * r, height: 2 * r };
        }
      } else if (tag === 'ellipse') {
        var rx = attrNum(this, 'rx'), ry = attrNum(this, 'ry');
        if (rx > 0 || ry > 0) {
          result = { x: attrNum(this, 'cx') - rx, y: attrNum(this, 'cy') - ry, width: 2 * rx, height: 2 * ry };
        }
      } else if (tag === 'line') {
        var x1 = attrNum(this, 'x1'), y1 = attrNum(this, 'y1');
        var x2 = attrNum(this, 'x2'), y2 = attrNum(this, 'y2');
        result = { x: Math.min(x1, x2), y: Math.min(y1, y2), width: Math.abs(x2 - x1), height: Math.abs(y2 - y1) };
      } else if (tag === 'path') {
        result = pathBBox(this.getAttribute && this.getAttribute('d'));
      } else if (tag === 'polygon' || tag === 'polyline') {
        var pts = (this.getAttribute && this.getAttribute('points')) || '';
        var m2 = pts.match(/-?\d*\.?\d+(?:e[-+]?\d+)?/g);
        var nums = m2 ? m2.map(Number) : null;
        if (nums && nums.length >= 2) {
          var minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
          for (var k = 0; k + 1 < nums.length; k += 2) {
            var px = nums[k], py = nums[k + 1];
            if (!isFinite(px) || !isFinite(py)) continue;
            if (px < minX) minX = px;
            if (py < minY) minY = py;
            if (px > maxX) maxX = px;
            if (py > maxY) maxY = py;
          }
          if (isFinite(minX) && maxX >= minX) {
            result = { x: minX, y: minY, width: maxX - minX, height: maxY - minY };
          }
        }
      }
      // 容器：合并子元素 bbox（每个子元素先应用自己的 transform）
      if (!result) {
        var box = null;
        var kids0 = this.children || [];
        for (var c0 = 0; c0 < kids0.length; c0++) {
          var cb;
          try { cb = kids0[c0].getBBox(); } catch (e) { cb = null; }
          if (!cb || (!cb.width && !cb.height)) continue;
          if (!box) { box = { x: cb.x, y: cb.y, width: cb.width, height: cb.height }; }
          else {
            var x2 = Math.max(box.x + box.width, cb.x + cb.width);
            var y2 = Math.max(box.y + box.height, cb.y + cb.height);
            box.x = Math.min(box.x, cb.x);
            box.y = Math.min(box.y, cb.y);
            box.width = x2 - box.x;
            box.height = y2 - box.y;
          }
        }
        result = box || { x: 0, y: 0, width: 0, height: 0 };
      }
    }

    if (!result) {
      var lines = text.split('\n');
      var w = 0;
      for (var i = 0; i < lines.length; i++) {
        var lw = measure(lines[i], fs, this.getAttribute && this.getAttribute('font-family'));
        if (lw > w) w = lw;
      }
      var h = Math.max(1, lines.length) * fs * 1.2;
      result = { x: 0, y: -fs * 0.8, width: w, height: h };
    }

    // 关键修复：对当前元素的 bbox 应用累积的 transform 链
    // 浏览器中 <g transform="translate(200,100)"> 的 getBBox 会返回全局坐标
    // 这里只应用当前元素自身的 transform（子元素已经在递归中应用过各自的）
    var myTransform = parseTransform(this);
    if (myTransform) {
      result = transformBBox(result, myTransform);
    }

    return result;
  };
  proto.getComputedTextLength = function () {
    var fs = fontSizeOf(this);
    return measure(this.textContent || '', fs, this.getAttribute && this.getAttribute('font-family'));
  };
  proto.getSubStringLength = function (start, len) {
    var fs = fontSizeOf(this);
    var t = (this.textContent || '').substr(start, len);
    return measure(t, fs, this.getAttribute && this.getAttribute('font-family'));
  };
  proto.getNumberOfChars = function () { return (this.textContent || '').length; };
  proto.getScreenCTM = function () {
    var m = cumulativeTransform(this);
    m.inverse = function () {
      var a = m.a, b = m.b, c = m.c, d = m.d, e = m.e, f = m.f;
      var det = a * d - b * c;
      if (!det || !isFinite(det)) return { a: 1, b: 0, c: 0, d: 1, e: 0, f: 0 };
      return {
        a: d / det, b: -b / det, c: -c / det, d: a / det,
        e: (c * f - d * e) / det, f: (b * e - a * f) / det
      };
    };
    m.multiply = function (other) {
      var a = m.a * other.a + m.c * other.b;
      var b = m.b * other.a + m.d * other.b;
      var c = m.a * other.c + m.c * other.d;
      var d = m.b * other.c + m.d * other.d;
      var e = m.a * other.e + m.c * other.f + m.e;
      var f = m.b * other.e + m.d * other.f + m.f;
      return { a, b, c, d, e, f, inverse: m.inverse, multiply: m.multiply };
    };
    return m;
  };
  proto.getCTM = proto.getScreenCTM;
  // ---- Canvas 2D → SVG mock ----
  // cytoscape mindmap 用 canvas 做文本度量 + 形状绘制；这里实现一个轻量 canvas2svg：
  // 所有绘制调用被追踪，路径累积为 SVG d 属性，fill/stroke 时发出 <path> 元素。
  // 同时保持 trace 数组供调试（__veilTrace）。
  var CanvasProto = L.HTMLCanvasElement && L.HTMLCanvasElement.prototype;
  if (CanvasProto) {
    // 初始化全局追踪数组
    globalThis.__veilTrace = globalThis.__veilTrace || [];
    globalThis.__veilSvgPaths = globalThis.__veilSvgPaths || [];
    globalThis.__veilGetCanvasTrace = function () { return globalThis.__veilTrace; };
    globalThis.__veilGetCanvasSvg = function () { return globalThis.__veilSvgPaths.join('\n'); };

    // 辅助：格式化数字
    function _f(n) { var v = Number(n); if (!isFinite(v)) v = 0; return (Math.round(v * 1000) / 1000).toString(); }

    function _ctx2dFactory() {
      var ctx = {
        canvas: null,
        // --- 状态属性 ---
        font: '10px sans-serif',
        fillStyle: '#000',
        strokeStyle: '#000',
        lineWidth: 1,
        globalAlpha: 1,
        shadowBlur: 0,
        shadowColor: 'rgba(0,0,0,0)',
        textAlign: 'start',
        textBaseline: 'alphabetic',
        lineCap: 'butt',
        lineJoin: 'miter',
        miterLimit: 10,
        lineDashOffset: 0,
        // --- 内部状态 ---
        _path: '',
        _hasPath: false,
        _stack: [],     // save/restore 栈
        _ctm: [1, 0, 0, 1, 0, 0], // 当前变换矩阵 [a b c d e f]
        _svgPaths: [],  // 本 ctx 产生的 SVG path 字符串

        // --- 追踪辅助 ---
        _trace: function (name, args) {
          var entry = [name];
          for (var i = 0; i < args.length; i++) {
            var a = args[i];
            if (typeof a === 'number') entry.push(Math.round(a * 1000) / 1000);
            else entry.push(a);
          }
          globalThis.__veilTrace.push(entry);
        },

        // --- 文本度量 ---
        measureText: function (t) {
          var m = /(\d+(?:\.\d+)?)px/.exec(this.font || '');
          var fs = m ? parseFloat(m[1]) : 16;
          var w = measure(String(t), fs, 'sans-serif');
          var result = {
            width: w,
            h: fs,           // cytoscape 读取此属性
            height: fs,
            actualBoundingBoxAscent: fs * 0.8,
            actualBoundingBoxDescent: fs * 0.2,
            actualBoundingBoxLeft: 0,
            actualBoundingBoxRight: w
          };
          this._trace('measureText', [String(t)]);
          return result;
        },

        // --- 状态保存/恢复 ---
        save: function () {
          this._trace('save', []);
          this._stack.push({
            font: this.font, fillStyle: this.fillStyle, strokeStyle: this.strokeStyle,
            lineWidth: this.lineWidth, globalAlpha: this.globalAlpha,
            textAlign: this.textAlign, textBaseline: this.textBaseline,
            lineCap: this.lineCap, lineJoin: this.lineJoin,
            miterLimit: this.miterLimit, lineDashOffset: this.lineDashOffset,
            ctm: this._ctm.slice(),
            path: this._path, hasPath: this._hasPath,
            globalCompositeOperation: this.globalCompositeOperation
          });
        },
        restore: function () {
          this._trace('restore', []);
          if (this._stack.length > 0) {
            var s = this._stack.pop();
            this.font = s.font; this.fillStyle = s.fillStyle; this.strokeStyle = s.strokeStyle;
            this.lineWidth = s.lineWidth; this.globalAlpha = s.globalAlpha;
            this.textAlign = s.textAlign; this.textBaseline = s.textBaseline;
            this.lineCap = s.lineCap; this.lineJoin = s.lineJoin;
            this.miterLimit = s.miterLimit; this.lineDashOffset = s.lineDashOffset;
            this._ctm = s.ctm;
            this._path = s.path; this._hasPath = s.hasPath;
            this.globalCompositeOperation = s.globalCompositeOperation;
          }
        },

        // --- 路径操作 ---
        beginPath: function () {
          this._trace('beginPath', []);
          this._path = '';
          this._hasPath = false;
        },
        closePath: function () {
          this._trace('closePath', []);
          this._path += ' Z';
        },
        moveTo: function (x, y) {
          this._trace('moveTo', [x, y]);
          this._path += (this._hasPath ? ' L' : ' M') + _f(x) + ' ' + _f(y);
          this._hasPath = true;
        },
        lineTo: function (x, y) {
          this._trace('lineTo', [x, y]);
          this._path += ' L' + _f(x) + ' ' + _f(y);
          this._hasPath = true;
        },
        arc: function (cx, cy, r, startAngle, endAngle, counterclockwise) {
          this._trace('arc', [cx, cy, r, startAngle, endAngle, counterclockwise]);
          // 用 SVG arc 命令近似完整圆
          var d = this._path;
          var startX = cx + r * Math.cos(startAngle);
          var startY = cy + r * Math.sin(startAngle);
          var endX = cx + r * Math.cos(endAngle);
          var endY = cy + r * Math.sin(endAngle);
          var largeArc = Math.abs(endAngle - startAngle) > Math.PI ? 1 : 0;
          var sweep = counterclockwise ? 0 : 1;
          d += (this._hasPath ? ' L' : ' M') + _f(startX) + ' ' + _f(startY);
          d += ' A' + _f(r) + ' ' + _f(r) + ' 0 ' + largeArc + ' ' + sweep + ' ' + _f(endX) + ' ' + _f(endY);
          this._path = d;
          this._hasPath = true;
        },
        ellipse: function (cx, cy, rx, ry, rotation, startAngle, endAngle, counterclockwise) {
          this._trace('ellipse', [cx, cy, rx, ry, rotation, startAngle, endAngle, counterclockwise]);
          var startX = cx + rx * Math.cos(startAngle);
          var startY = cy + ry * Math.sin(startAngle);
          var endX = cx + rx * Math.cos(endAngle);
          var endY = cy + ry * Math.sin(endAngle);
          var largeArc = Math.abs(endAngle - startAngle) > Math.PI ? 1 : 0;
          var sweep = counterclockwise ? 0 : 1;
          this._path += (this._hasPath ? ' L' : ' M') + _f(startX) + ' ' + _f(startY);
          this._path += ' A' + _f(rx) + ' ' + _f(ry) + ' ' + _f(rotation) + ' ' + largeArc + ' ' + sweep + ' ' + _f(endX) + ' ' + _f(endY);
          this._hasPath = true;
        },
        bezierCurveTo: function (cp1x, cp1y, cp2x, cp2y, x, y) {
          this._trace('bezierCurveTo', [cp1x, cp1y, cp2x, cp2y, x, y]);
          this._path += ' C' + _f(cp1x) + ' ' + _f(cp1y) + ' ' + _f(cp2x) + ' ' + _f(cp2y) + ' ' + _f(x) + ' ' + _f(y);
          this._hasPath = true;
        },
        quadraticCurveTo: function (cpx, cpy, x, y) {
          this._trace('quadraticCurveTo', [cpx, cpy, x, y]);
          this._path += ' Q' + _f(cpx) + ' ' + _f(cpy) + ' ' + _f(x) + ' ' + _f(y);
          this._hasPath = true;
        },
        rect: function (x, y, w, h) {
          this._trace('rect', [x, y, w, h]);
          // 添加为子路径（先 closePath 前的现有路径保持）
          this._path += (this._hasPath ? ' M' : ' M') + _f(x) + ' ' + _f(y);
          this._path += ' L' + _f(x + w) + ' ' + _f(y);
          this._path += ' L' + _f(x + w) + ' ' + _f(y + h);
          this._path += ' L' + _f(x) + ' ' + _f(y + h) + ' Z';
          this._hasPath = true;
        },

        // --- 填充/描边 ---
        _emitSvgPath: function (d, fill, stroke, sw, extra) {
          if (!d || !d.trim()) return;
          var attrs = [];
          attrs.push('d="' + d.trim() + '"');
          if (fill && fill !== 'none') attrs.push('fill="' + fill + '"');
          else attrs.push('fill="none"');
          if (stroke && stroke !== 'none') {
            attrs.push('stroke="' + stroke + '"');
            attrs.push('stroke-width="' + _f(sw || 1) + '"');
          }
          if (extra) attrs.push(extra);
          var svg = '<path ' + attrs.join(' ') + '/>';
          this._svgPaths.push(svg);
          globalThis.__veilSvgPaths.push(svg);
        },

        fill: function () {
          this._trace('fill', []);
          this._emitSvgPath(this._path, this.fillStyle, 'none', 0);
        },
        stroke: function () {
          this._trace('stroke', []);
          this._emitSvgPath(this._path, 'none', this.strokeStyle, this.lineWidth);
        },
        fillRect: function (x, y, w, h) {
          this._trace('fillRect', [x, y, w, h]);
          // 直接输出 SVG rect 元素
          var svg = '<rect x="' + _f(x) + '" y="' + _f(y) + '" width="' + _f(w) + '" height="' + _f(h) + '" fill="' + this.fillStyle + '"/>';
          this._svgPaths.push(svg);
          globalThis.__veilSvgPaths.push(svg);
        },
        strokeRect: function (x, y, w, h) {
          this._trace('strokeRect', [x, y, w, h]);
          var svg = '<rect x="' + _f(x) + '" y="' + _f(y) + '" width="' + _f(w) + '" height="' + _f(h) + '" fill="none" stroke="' + this.strokeStyle + '" stroke-width="' + _f(this.lineWidth) + '"/>';
          this._svgPaths.push(svg);
          globalThis.__veilSvgPaths.push(svg);
        },
        clearRect: function () {
          this._trace('clearRect', Array.prototype.slice.call(arguments));
          // no-op for SVG
        },
        clip: function () {
          this._trace('clip', []);
          // 简化：忽略 clip
        },

        // --- 变换 ---
        translate: function (x, y) {
          this._trace('translate', [x, y]);
          var c = this._ctm;
          var nc = [1, 0, 0, 1, x, y];
          this._ctm = [
            c[0] * nc[0] + c[2] * nc[1],
            c[1] * nc[0] + c[3] * nc[1],
            c[0] * nc[2] + c[2] * nc[3],
            c[1] * nc[2] + c[3] * nc[3],
            c[0] * nc[4] + c[2] * nc[5] + c[4],
            c[1] * nc[4] + c[3] * nc[5] + c[5]
          ];
        },
        scale: function (sx, sy) {
          this._trace('scale', [sx, sy]);
          var c = this._ctm;
          this._ctm = [c[0] * sx, c[1] * sx, c[2] * sy, c[3] * sy, c[4], c[5]];
        },
        rotate: function (angle) {
          this._trace('rotate', [angle]);
          var cos = Math.cos(angle), sin = Math.sin(angle);
          var c = this._ctm;
          var nc = [cos, sin, -sin, cos, 0, 0];
          this._ctm = [
            c[0] * nc[0] + c[2] * nc[1],
            c[1] * nc[0] + c[3] * nc[1],
            c[0] * nc[2] + c[2] * nc[3],
            c[1] * nc[2] + c[3] * nc[3],
            c[4], c[5]
          ];
        },
        setTransform: function (a, b, c, d, e, f) {
          this._trace('setTransform', [a, b, c, d, e, f]);
          this._ctm = [a, b, c, d, e, f];
        },
        transform: function (a, b, c, d, e, f) {
          this._trace('transform', [a, b, c, d, e, f]);
          var c0 = this._ctm;
          var nc = [a, b, c, d, e, f];
          this._ctm = [
            c0[0] * nc[0] + c0[2] * nc[1],
            c0[1] * nc[0] + c0[3] * nc[1],
            c0[0] * nc[2] + c0[2] * nc[3],
            c0[1] * nc[2] + c0[3] * nc[3],
            c0[0] * nc[4] + c0[2] * nc[5] + c0[4],
            c0[1] * nc[4] + c0[3] * nc[5] + c0[5]
          ];
        },

        // --- 文本绘制 ---
        fillText: function (text, x, y, maxWidth) {
          this._trace('fillText', [String(text), x, y, maxWidth]);
          var m = /(\d+(?:\.\d+)?)px/.exec(this.font || '');
          var fs = m ? parseFloat(m[1]) : 16;
          var svg = '<text x="' + _f(x) + '" y="' + _f(y) + '" fill="' + this.fillStyle + '" font-size="' + _f(fs) + '" font-family="sans-serif">' + String(text).replace(/[<>&]/g, function (c) { return c === '<' ? '&lt;' : c === '>' ? '&gt;' : '&amp;'; }) + '</text>';
          this._svgPaths.push(svg);
          globalThis.__veilSvgPaths.push(svg);
        },
        strokeText: function () {
          this._trace('strokeText', Array.prototype.slice.call(arguments));
          // 简化：不输出 stroke 文本
        },

        // --- 其他 ---
        drawImage: function () {
          this._trace('drawImage', Array.prototype.slice.call(arguments));
        },
        putImageData: function () {
          this._trace('putImageData', Array.prototype.slice.call(arguments));
        },
        getImageData: function (x, y, w, h) {
          this._trace('getImageData', [x, y, w, h]);
          return { width: w || 0, height: h || 0, data: new Array((w || 0) * (h || 0) * 4) };
        },
        createLinearGradient: function () { return { addColorStop: function () {} }; },
        createRadialGradient: function () { return { addColorStop: function () {} }; },
        createPattern: function () { return {}; },
        setLineDash: function () {},
        getLineDash: function () { return []; },
        createImageData: function (w, h) {
          return { width: w || 0, height: h || 0, data: new Array((w || 0) * (h || 0) * 4) };
        }
      };
      return ctx;
    }

    CanvasProto.getContext = function (type) {
      if (type !== '2d') return null;
      var c = _ctx2dFactory();
      c.canvas = this;
      return c;
    };
  }

  // HTML 元素也走同一套度量（flowchart 默认 htmlLabels 依赖 getBoundingClientRect 定尺寸）
  proto.getBoundingClientRect = function () {
    var fs = fontSizeOf(this);
    var text = (this.textContent || '').trim() || '';
    var lines = text.split('\n');
    var w = 0;
    for (var i = 0; i < lines.length; i++) {
      var lw = measure(lines[i], fs, this.getAttribute && this.getAttribute('font-family'));
      if (lw > w) w = lw;
    }
    var h = Math.max(1, lines.length) * fs * 1.2;
    return { x: 0, y: 0, width: w, height: h, top: 0, left: 0, right: w, bottom: h };
  };

  // cytoscape 用 container.clientWidth/clientHeight 计算画布尺寸
  // linkedom 默认没有这两个属性，会返回 undefined → NaN → makeBoundingBox 返回 undefined
  if (!('clientWidth' in proto)) {
    Object.defineProperty(proto, 'clientWidth', {
      get: function () {
        var s = this.style && this.style.width ? parseFloat(this.style.width) : 0;
        if (!isFinite(s) || s <= 0) s = 800; // cytoscape 默认容器尺寸
        return s;
      }
    });
  }
  if (!('clientHeight' in proto)) {
    Object.defineProperty(proto, 'clientHeight', {
      get: function () {
        var s = this.style && this.style.height ? parseFloat(this.style.height) : 0;
        if (!isFinite(s) || s <= 0) s = 600;
        return s;
      }
    });
  }
  // cytoscape/mermaid 也可能读 scrollWidth/scrollHeight/offsetWidth/offsetHeight
  // 没有这些的话会从 style 里取 NaN，算出巨大的边界
  ['scrollWidth', 'offsetWidth'].forEach(function(name) {
    if (!(name in proto)) {
      Object.defineProperty(proto, name, {
        get: function () {
          var cw = this.clientWidth || 800;
          // 加上 padding
          var gs = globalThis.getComputedStyle(this);
          var pl = parseFloat(gs.getPropertyValue('paddingLeft')) || 0;
          var pr = parseFloat(gs.getPropertyValue('paddingRight')) || 0;
          return cw + pl + pr;
        }
      });
    }
  });
  ['scrollHeight', 'offsetHeight'].forEach(function(name) {
    if (!(name in proto)) {
      Object.defineProperty(proto, name, {
        get: function () {
          var ch = this.clientHeight || 600;
          var gs = globalThis.getComputedStyle(this);
          var pt = parseFloat(gs.getPropertyValue('paddingTop')) || 0;
          var pb = parseFloat(gs.getPropertyValue('paddingBottom')) || 0;
          return ch + pt + pb;
        }
      });
    }
  });
  // ---- 宿主缺失的标准全局补齐（QuickJS 不提供，Node 上下文里也可能缺）----
  if (!globalThis.btoa) {
    var B64 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
    globalThis.btoa = function (input) {
      var s = String(input);
      var out = '';
      for (var i = 0; i < s.length; i += 3) {
        var c1 = s.charCodeAt(i), c2 = s.charCodeAt(i + 1), c3 = s.charCodeAt(i + 2);
        out += B64.charAt(c1 >> 2);
        out += B64.charAt(((c1 & 3) << 4) | ((isNaN(c2) ? 0 : c2) >> 4));
        out += isNaN(c2) ? '=' : B64.charAt(((c2 & 15) << 2) | ((isNaN(c3) ? 0 : c3) >> 6));
        out += isNaN(c3) ? '=' : B64.charAt(c3 & 63);
      }
      return out;
    };
  }
  if (!globalThis.atob) {
    var B64D = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
    globalThis.atob = function (input) {
      var s = String(input).replace(/[^A-Za-z0-9+/]/g, '');
      var out = '';
      for (var i = 0; i < s.length; i += 4) {
        var n1 = B64D.indexOf(s.charAt(i)), n2 = B64D.indexOf(s.charAt(i + 1));
        var n3 = B64D.indexOf(s.charAt(i + 2)), n4 = B64D.indexOf(s.charAt(i + 3));
        out += String.fromCharCode((n1 << 2) | (n2 >> 4));
        if (n3 >= 0) out += String.fromCharCode(((n2 & 15) << 4) | (n3 >> 2));
        if (n4 >= 0) out += String.fromCharCode(((n3 & 3) << 6) | n4);
      }
      return out;
    };
  }
  if (!globalThis.structuredClone) {
    // mermaid 只用它深拷贝纯数据配置，JSON 往返足够
    globalThis.structuredClone = function (v) { return JSON.parse(JSON.stringify(v)); };
  }
  if (!globalThis.crypto) globalThis.crypto = {};
  if (!globalThis.crypto.getRandomValues) {
    globalThis.crypto.getRandomValues = function (arr) {
      for (var i = 0; i < arr.length; i++) arr[i] = Math.floor(Math.random() * 256);
      return arr;
    };
  }

  // 统一整理 Mermaid 的 standalone SVG。viewBox 仍完全由 Mermaid 布局计算；
  // 数值 width/height 只提供浏览器 <img> 所需的固有尺寸，避免默认 300×150。
  globalThis.__veilFinalizeSvg = function (svg) {
    var parsed = new WrappedParser().parseFromString(String(svg), 'image/svg+xml');
    var root = parsed.querySelector('svg');
    if (!root) return String(svg);

    var viewBox = (root.getAttribute('viewBox') || root.getAttribute('viewbox') || '').match(/[+-]?(?:\d*\.\d+|\d+\.?)(?:[eE][+-]?\d+)?/g) || [];
    if (viewBox.length === 4) {
      var width = Number(viewBox[2]), height = Number(viewBox[3]);
      if (isFinite(width) && isFinite(height) && width > 0 && height > 0) {
        root.setAttribute('viewBox', viewBox.join(' '));
        root.removeAttribute('viewbox');
        root.setAttribute('width', String(width));
        root.setAttribute('height', String(height));
      }
    }

    // htmlLabels:false 使用通用 rect.background，而 Mermaid 的图型专用主题
    // 只命中 relationshipLabelBox/reqLabelBox。补回原有类即可使用当前主题的
    // 前景/背景规则，不写死颜色，也不改变布局或隐藏任何内容。
    var labelBoxClass = {
      er: 'relationshipLabelBox',
      requirement: 'reqLabelBox'
    }[root.getAttribute('aria-roledescription') || ''];
    if (labelBoxClass) {
      var backgrounds = root.querySelectorAll('.edgeLabel rect.background');
      for (var i = 0; i < backgrounds.length; i++) {
        var classes = backgrounds[i].getAttribute('class') || '';
        if (!new RegExp('(^|\\s)' + labelBoxClass + '(?:\\s|$)').test(classes)) {
          backgrounds[i].setAttribute('class', classes + ' ' + labelBoxClass);
        }
      }
    }

    // dagre 在极短边上可能生成控制点全部重合的 cubic，随后再补一个重复 L。
    // 这些段的可见几何等价于一条直线，但零长度末端会让浏览器/ resvg 的
    // marker orient="auto" 退回 0°。只规范化这种退化段，保留有效曲线和端点。
    var number = '[+-]?(?:\\d*\\.\\d+|\\d+\\.?)(?:[eE][+-]?\\d+)?';
    var cubic = new RegExp('C\\s*(' + number + ')[\\s,]+(' + number + ')[\\s,]+(' + number + ')[\\s,]+(' + number + ')[\\s,]+(' + number + ')[\\s,]+(' + number + ')', 'g');
    var duplicateLine = new RegExp('L\\s*(' + number + ')[\\s,]+(' + number + ')\\s*L\\s*(' + number + ')[\\s,]+(' + number + ')', 'g');
    var paths = root.querySelectorAll('path');
    for (var p = 0; p < paths.length; p++) {
      if (!paths[p].getAttribute('marker-end') && !paths[p].getAttribute('marker-start')) continue;
      var d = paths[p].getAttribute('d') || '';
      d = d.replace(cubic, function (segment, x1, y1, x2, y2, x, y) {
        if (Number(x1) === Number(x2) && Number(x2) === Number(x) &&
            Number(y1) === Number(y2) && Number(y2) === Number(y)) {
          return 'L' + x + ',' + y;
        }
        return segment;
      });
      var previous;
      do {
        previous = d;
        d = d.replace(duplicateLine, function (segments, x1, y1, x2, y2) {
          return Number(x1) === Number(x2) && Number(y1) === Number(y2)
            ? 'L' + x1 + ',' + y1 : segments;
        });
      } while (d !== previous);
      paths[p].setAttribute('d', d);
    }
    return root.outerHTML;
  };

})();
