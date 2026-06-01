/* Lattice — virtualized canvas spreadsheet front-end. Vanilla JS, offline. */
(function () {
  "use strict";

  // ---------- Fixed grid metrics ----------
  var COL_W = 92;       // column width (px)
  var ROW_H = 24;       // body row height (px)
  var HEAD_H = 24;      // column-header band height (px)
  var HEAD_W = 46;      // row-header band width (px)
  var N_ROWS = 1000;    // logical rows
  var N_COLS = 52;      // logical columns (A..AZ)

  // ---------- Backend bridge ----------
  var TAURI = (typeof window !== "undefined" && window.__TAURI__ && window.__TAURI__.core) ? window.__TAURI__.core : null;
  function invoke(name, args) {
    if (!TAURI) return Promise.reject(new Error("no-backend"));
    return TAURI.invoke(name, args || {});
  }

  // ---------- Local model ----------
  // model: Map "A1" -> { input, display }
  var model = new Map();
  var sheetIndex = 0;
  var sheetName = "Sheet1";

  // Active selection (0-based)
  var sel = { col: 0, row: 0 };

  // Scroll origin (top-left logical cell partially shown)
  var scroll = { x: 0, y: 0 };

  var editing = false;       // is the in-cell editor open
  var editFromChar = false;  // editor opened by typing (replace) vs F2 (keep)

  // ---------- A1 helpers (mirror sheet-core/cellref.rs) ----------
  function colToLetters(col) {
    var s = "";
    for (;;) {
      s = String.fromCharCode(65 + (col % 26)) + s;
      if (col < 26) break;
      col = Math.floor(col / 26) - 1;
    }
    return s;
  }
  function a1(col, row) { return colToLetters(col) + (row + 1); }

  function isNumericDisplay(rec) {
    if (!rec) return false;
    var inp = (rec.input == null ? "" : String(rec.input)).trim();
    var disp = (rec.display == null ? "" : String(rec.display)).trim();
    if (disp === "") return false;
    // Errors / booleans are centered-left like text; numbers right-align.
    if (disp.charAt(0) === "#") return false;
    if (disp === "TRUE" || disp === "FALSE") return false;
    if (inp.charAt(0) === "=") {
      // formula: right-align when the computed display reads as a number
      return /^-?[\d,]*\.?\d+(?:[eE][-+]?\d+)?%?$/.test(disp);
    }
    return /^-?[\d,]*\.?\d+(?:[eE][-+]?\d+)?%?$/.test(inp);
  }

  // ---------- DOM refs ----------
  var canvas = document.getElementById("grid-canvas");
  var ctx = canvas.getContext("2d");
  var wrap = document.getElementById("grid-wrap");
  var scroller = document.getElementById("scroller");
  var spacer = document.getElementById("scroll-spacer");
  var editor = document.getElementById("cell-editor");
  var fxInput = document.getElementById("formula-input");
  var cellRefEl = document.getElementById("cell-ref");
  var sheetNameEl = document.getElementById("sheet-name");
  var statusSel = document.getElementById("status-sel");
  var statusMsg = document.getElementById("status-msg");
  var statusMode = document.getElementById("status-mode");

  var dpr = Math.max(1, window.devicePixelRatio || 1);

  // ---------- Sizing ----------
  function viewport() {
    // pixel area available for the canvas (CSS px)
    return { w: wrap.clientWidth, h: wrap.clientHeight };
  }

  function resizeCanvas() {
    dpr = Math.max(1, window.devicePixelRatio || 1);
    var vp = viewport();
    canvas.style.width = vp.w + "px";
    canvas.style.height = vp.h + "px";
    canvas.width = Math.round(vp.w * dpr);
    canvas.height = Math.round(vp.h * dpr);
    // Spacer reflects the full logical extent so the scrollbar is real.
    spacer.style.width = (HEAD_W + N_COLS * COL_W) + "px";
    spacer.style.height = (HEAD_H + N_ROWS * ROW_H) + "px";
    draw();
  }

  // ---------- Virtualization: scroll -> visible cell range ----------
  // The scroller's scrollLeft/scrollTop are pixel offsets into a logical surface
  // sized [HEAD_W + N_COLS*COL_W] x [HEAD_H + N_ROWS*ROW_H]. The frozen header
  // band occupies the first HEAD_W / HEAD_H pixels and never scrolls. Body cells
  // begin at that offset, so the first visible *data* column is
  //   firstCol = floor(scrollLeft / COL_W)
  // and we draw columns until the right edge of the viewport is covered. Each
  // column c is painted at screen-x = HEAD_W + c*COL_W - scrollLeft. Rows work
  // identically with ROW_H / scrollTop. Only cells in [firstCol..lastCol] x
  // [firstRow..lastRow] are ever touched — that is the whole virtualization.
  function visibleRange() {
    var vp = viewport();
    var sx = scroll.x, sy = scroll.y;
    var firstCol = Math.floor(sx / COL_W);
    var firstRow = Math.floor(sy / ROW_H);
    var bodyW = vp.w - HEAD_W;
    var bodyH = vp.h - HEAD_H;
    var lastCol = Math.floor((sx + bodyW) / COL_W);
    var lastRow = Math.floor((sy + bodyH) / ROW_H);
    firstCol = Math.max(0, firstCol);
    firstRow = Math.max(0, firstRow);
    lastCol = Math.min(N_COLS - 1, lastCol);
    lastRow = Math.min(N_ROWS - 1, lastRow);
    return { firstCol: firstCol, lastCol: lastCol, firstRow: firstRow, lastRow: lastRow };
  }

  // screen pixel for the left edge of a logical column (body coords)
  function colX(col) { return HEAD_W + col * COL_W - scroll.x; }
  function rowY(row) { return HEAD_H + row * ROW_H - scroll.y; }

  // ---------- Drawing ----------
  function px(v) { return Math.round(v) + 0.5; } // crisp 1px lines

  function draw() {
    var vp = viewport();
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, vp.w, vp.h);
    ctx.fillStyle = "#ffffff";
    ctx.fillRect(0, 0, vp.w, vp.h);

    var r = visibleRange();
    drawBodyCells(r, vp);
    drawGridLines(r, vp);
    drawSelection(r);
    drawHeaders(r, vp);
  }

  function drawBodyCells(r, vp) {
    ctx.textBaseline = "middle";
    ctx.font = '13px -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif';
    var clipLeft = HEAD_W, clipTop = HEAD_H;
    for (var row = r.firstRow; row <= r.lastRow; row++) {
      var y = rowY(row);
      for (var col = r.firstCol; col <= r.lastCol; col++) {
        var rec = model.get(a1(col, row));
        if (!rec || rec.display == null || rec.display === "") continue;
        var x = colX(col);
        // clip text to the cell, never spilling under the frozen headers
        var cellLeft = Math.max(x, clipLeft);
        var cellRight = x + COL_W;
        if (cellRight <= clipLeft || y + ROW_H <= clipTop) continue;
        ctx.save();
        ctx.beginPath();
        ctx.rect(cellLeft, Math.max(y, clipTop), cellRight - cellLeft - 1, ROW_H);
        ctx.clip();
        var isErr = String(rec.display).charAt(0) === "#";
        ctx.fillStyle = isErr ? "#b42318" : "#1f2328";
        var cy = y + ROW_H / 2 + 0.5;
        if (isNumericDisplay(rec)) {
          ctx.textAlign = "right";
          ctx.fillText(rec.display, x + COL_W - 6, cy);
        } else {
          ctx.textAlign = "left";
          ctx.fillText(rec.display, x + 6, cy);
        }
        ctx.restore();
      }
    }
  }

  function drawGridLines(r, vp) {
    ctx.strokeStyle = "#e7eaed";
    ctx.lineWidth = 1;
    ctx.beginPath();
    var topClip = HEAD_H, leftClip = HEAD_W;
    // vertical lines
    for (var col = r.firstCol; col <= r.lastCol + 1; col++) {
      var x = colX(col);
      if (x < leftClip) continue;
      if (x > vp.w) break;
      ctx.moveTo(px(x), topClip);
      ctx.lineTo(px(x), vp.h);
    }
    // horizontal lines
    for (var row = r.firstRow; row <= r.lastRow + 1; row++) {
      var y = rowY(row);
      if (y < topClip) continue;
      if (y > vp.h) break;
      ctx.moveTo(leftClip, px(y));
      ctx.lineTo(vp.w, px(y));
    }
    ctx.stroke();
  }

  function drawSelection(r) {
    if (sel.col < r.firstCol || sel.col > r.lastCol || sel.row < r.firstRow || sel.row > r.lastRow) {
      return; // active cell scrolled out of view
    }
    var x = colX(sel.col), y = rowY(sel.row);
    // soft fill
    ctx.save();
    ctx.beginPath();
    ctx.rect(HEAD_W, HEAD_H, viewport().w - HEAD_W, viewport().h - HEAD_H);
    ctx.clip();
    ctx.fillStyle = "rgba(26,115,232,0.10)";
    ctx.fillRect(x, y, COL_W, ROW_H);
    // crisp 2px border
    ctx.strokeStyle = "#1a73e8";
    ctx.lineWidth = 2;
    ctx.strokeRect(px(x) - 0.5, px(y) - 0.5, COL_W, ROW_H);
    ctx.restore();
  }

  function drawHeaders(r, vp) {
    // Column header band
    ctx.fillStyle = "#f6f7f9";
    ctx.fillRect(0, 0, vp.w, HEAD_H);
    // Row header band
    ctx.fillRect(0, 0, HEAD_W, vp.h);
    // Corner box
    ctx.fillStyle = "#eef0f3";
    ctx.fillRect(0, 0, HEAD_W, HEAD_H);

    ctx.textBaseline = "middle";
    ctx.font = '12px -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif';

    // active-header highlight
    var aHx = colX(sel.col), aHy = rowY(sel.row);

    // column labels
    ctx.save();
    ctx.beginPath();
    ctx.rect(HEAD_W, 0, vp.w - HEAD_W, HEAD_H);
    ctx.clip();
    for (var col = r.firstCol; col <= r.lastCol; col++) {
      var x = colX(col);
      if (col === sel.col) {
        ctx.fillStyle = "#e8effb";
        ctx.fillRect(x, 0, COL_W, HEAD_H);
      }
      ctx.fillStyle = (col === sel.col) ? "#1a56db" : "#57606a";
      ctx.textAlign = "center";
      ctx.fillText(colToLetters(col), x + COL_W / 2, HEAD_H / 2 + 0.5);
    }
    ctx.restore();

    // row labels
    ctx.save();
    ctx.beginPath();
    ctx.rect(0, HEAD_H, HEAD_W, vp.h - HEAD_H);
    ctx.clip();
    for (var row = r.firstRow; row <= r.lastRow; row++) {
      var y = rowY(row);
      if (row === sel.row) {
        ctx.fillStyle = "#e8effb";
        ctx.fillRect(0, y, HEAD_W, ROW_H);
      }
      ctx.fillStyle = (row === sel.row) ? "#1a56db" : "#57606a";
      ctx.textAlign = "center";
      ctx.fillText(String(row + 1), HEAD_W / 2, y + ROW_H / 2 + 0.5);
    }
    ctx.restore();

    // header separator lines
    ctx.strokeStyle = "#d8dce1";
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(0, px(HEAD_H)); ctx.lineTo(vp.w, px(HEAD_H));
    ctx.moveTo(px(HEAD_W), 0); ctx.lineTo(px(HEAD_W), vp.h);
    ctx.stroke();
    void aHx; void aHy;
  }

  // ---------- Selection + scrolling ----------
  function setSelection(col, row, opts) {
    col = Math.max(0, Math.min(N_COLS - 1, col));
    row = Math.max(0, Math.min(N_ROWS - 1, row));
    sel.col = col; sel.row = row;
    var ref = a1(col, row);
    cellRefEl.textContent = ref;
    statusSel.textContent = ref;
    var rec = model.get(ref);
    fxInput.value = rec ? (rec.input == null ? "" : rec.input) : "";
    if (!opts || opts.reveal !== false) ensureVisible(col, row);
    draw();
  }

  function ensureVisible(col, row) {
    var vp = viewport();
    var bodyW = vp.w - HEAD_W;
    var bodyH = vp.h - HEAD_H;
    var cellLeft = col * COL_W;
    var cellTop = row * ROW_H;
    var nx = scroll.x, ny = scroll.y;
    if (cellLeft < scroll.x) nx = cellLeft;
    else if (cellLeft + COL_W > scroll.x + bodyW) nx = cellLeft + COL_W - bodyW;
    if (cellTop < scroll.y) ny = cellTop;
    else if (cellTop + ROW_H > scroll.y + bodyH) ny = cellTop + ROW_H - bodyH;
    if (nx !== scroll.x || ny !== scroll.y) {
      scroller.scrollLeft = nx;
      scroller.scrollTop = ny;
      // scroll handler will sync scroll.{x,y} and redraw
    }
  }

  // map a pointer event (in grid-wrap coords) to a logical cell, or null on headers
  function hitCell(clientX, clientY) {
    var rect = wrap.getBoundingClientRect();
    var px2 = clientX - rect.left;
    var py2 = clientY - rect.top;
    if (px2 < HEAD_W || py2 < HEAD_H) return null;
    var col = Math.floor((px2 - HEAD_W + scroll.x) / COL_W);
    var row = Math.floor((py2 - HEAD_H + scroll.y) / ROW_H);
    if (col < 0 || col >= N_COLS || row < 0 || row >= N_ROWS) return null;
    return { col: col, row: row };
  }

  // ---------- In-cell editor ----------
  function openEditor(seedChar) {
    var vp = viewport();
    var x = colX(sel.col), y = rowY(sel.row);
    // keep editor within the body region
    if (x < HEAD_W || y < HEAD_H || x > vp.w || y > vp.h) {
      ensureVisible(sel.col, sel.row);
      x = colX(sel.col); y = rowY(sel.row);
    }
    editing = true;
    editor.hidden = false;
    editor.style.left = x + "px";
    editor.style.top = y + "px";
    editor.style.width = COL_W + "px";
    editor.style.height = ROW_H + "px";
    var rec = model.get(a1(sel.col, sel.row));
    if (seedChar != null) {
      editor.value = seedChar;
      editFromChar = true;
    } else {
      editor.value = rec ? (rec.input == null ? "" : rec.input) : "";
      editFromChar = false;
    }
    editor.focus();
    if (seedChar == null) editor.select();
    else {
      var end = editor.value.length;
      try { editor.setSelectionRange(end, end); } catch (e) {}
    }
  }

  function closeEditor(commitDir) {
    if (!editing) return;
    var commit = commitDir !== null && commitDir !== undefined;
    var value = editor.value;
    editing = false;
    editor.hidden = true;
    editor.value = "";
    if (commit) {
      commitCell(sel.col, sel.row, value, commitDir);
    }
    scroller.focus();
  }

  function commitCell(col, row, input, dir) {
    var ref = a1(col, row);
    applyLocal(ref, input);
    statusMsg.textContent = "Calculating…";
    invoke("set_cell", { sheet: sheetIndex, a1: ref, input: input })
      .then(function (res) {
        if (res && res.cells) patchCells(res.cells);
        statusMsg.textContent = "Ready";
      })
      .catch(function () {
        // offline / demo: do a tiny local recompute so totals feel alive
        localRecompute();
        statusMsg.textContent = TAURI ? "Edit failed" : "Ready";
      })
      .then(function () {
        // refresh formula bar if active cell unchanged
        refreshActiveInputBar();
        moveAfterCommit(dir);
      });
  }

  function moveAfterCommit(dir) {
    if (dir === "down") setSelection(sel.col, sel.row + 1);
    else if (dir === "right") setSelection(sel.col + 1, sel.row);
    else if (dir === "up") setSelection(sel.col, sel.row - 1);
    else if (dir === "left") setSelection(sel.col - 1, sel.row);
    else setSelection(sel.col, sel.row);
  }

  function refreshActiveInputBar() {
    var rec = model.get(a1(sel.col, sel.row));
    fxInput.value = rec ? (rec.input == null ? "" : rec.input) : "";
  }

  // Set local input immediately (display will be patched by backend response).
  function applyLocal(ref, input) {
    if (input === "" || input == null) {
      model.delete(ref);
    } else {
      var prev = model.get(ref) || {};
      model.set(ref, { input: input, display: prev.display != null ? prev.display : input });
    }
    draw();
  }

  // Patch display (and input when provided) for a set of cells.
  function patchCells(cells) {
    for (var i = 0; i < cells.length; i++) {
      var c = cells[i];
      if (!c || !c.a1) continue;
      var existing = model.get(c.a1) || {};
      var input = (c.input != null) ? c.input : existing.input;
      var display = (c.display != null) ? c.display : existing.display;
      if ((display == null || display === "") && (input == null || input === "")) {
        model.delete(c.a1);
      } else {
        model.set(c.a1, { input: input == null ? "" : input, display: display == null ? "" : display });
      }
    }
    draw();
  }

  // ---------- Offline demo recompute (only used when no backend) ----------
  // Recomputes any local cell whose input is =SUM(range); enough to keep the
  // demo's total cell honest as you edit. Real recalculation lives in the engine.
  function localRecompute() {
    model.forEach(function (rec, ref) {
      if (!rec || typeof rec.input !== "string") return;
      var m = /^=SUM\(([A-Z]+\d+):([A-Z]+\d+)\)$/i.exec(rec.input.trim());
      if (!m) return;
      var a = parseA1(m[1]), b = parseA1(m[2]);
      if (!a || !b) return;
      var sum = 0;
      var c0 = Math.min(a.col, b.col), c1 = Math.max(a.col, b.col);
      var r0 = Math.min(a.row, b.row), r1 = Math.max(a.row, b.row);
      for (var rr = r0; rr <= r1; rr++) {
        for (var cc = c0; cc <= c1; cc++) {
          var cell = model.get(a1(cc, rr));
          if (cell && cell.input != null) {
            var n = parseFloat(String(cell.input).replace(/,/g, ""));
            if (!isNaN(n) && /^-?[\d,]*\.?\d+$/.test(String(cell.input).trim())) sum += n;
          }
        }
      }
      rec.display = formatNum(sum);
    });
    draw();
  }

  function parseA1(s) {
    var m = /^\$?([A-Za-z]+)\$?(\d+)$/.exec(String(s).trim());
    if (!m) return null;
    var letters = m[1].toUpperCase();
    var col = 0;
    for (var i = 0; i < letters.length; i++) col = col * 26 + (letters.charCodeAt(i) - 64);
    col -= 1;
    var row = parseInt(m[2], 10) - 1;
    if (col < 0 || row < 0) return null;
    return { col: col, row: row };
  }

  function formatNum(n) {
    if (!isFinite(n)) return "#NUM!";
    if (n === Math.trunc(n) && Math.abs(n) < 1e15) return String(n);
    return String(Math.round(n * 1e10) / 1e10);
  }

  // ---------- Snapshot loading ----------
  function loadSnapshot(snap) {
    model.clear();
    if (snap && snap.name) { sheetName = snap.name; sheetNameEl.textContent = snap.name; }
    if (snap && snap.cells) {
      for (var i = 0; i < snap.cells.length; i++) {
        var c = snap.cells[i];
        if (!c || !c.a1) continue;
        model.set(c.a1, {
          input: c.input == null ? "" : c.input,
          display: c.display == null ? "" : c.display
        });
      }
    }
    setSelection(0, 0, { reveal: false });
    scroller.scrollLeft = 0; scroller.scrollTop = 0;
    scroll.x = 0; scroll.y = 0;
    draw();
  }

  // Build a lively demo workbook for plain-browser / screenshot use.
  function demoSnapshot() {
    var cells = [];
    function put(ref, input, display) { cells.push({ a1: ref, input: input, display: display == null ? input : display }); }
    var headers = ["Region", "Q1", "Q2", "Q3", "Q4", "Total"];
    var cols = ["A", "B", "C", "D", "E", "F"];
    for (var i = 0; i < headers.length; i++) put(cols[i] + "1", headers[i]);
    var rows = [
      ["North", 1240, 1310, 1455, 1502],
      ["South", 980, 1025, 1180, 1240],
      ["East", 1520, 1490, 1610, 1725],
      ["West", 870, 932, 1015, 1102],
      ["Central", 1105, 1188, 1247, 1330]
    ];
    var r;
    for (r = 0; r < rows.length; r++) {
      var rowNum = r + 2;
      put("A" + rowNum, rows[r][0]);
      var tot = 0;
      for (var q = 1; q <= 4; q++) {
        var v = rows[r][q];
        put(cols[q] + rowNum, String(v));
        tot += v;
      }
      put("F" + rowNum, "=SUM(B" + rowNum + ":E" + rowNum + ")", String(tot));
    }
    // column totals row
    var totRow = rows.length + 2; // row 7
    put("A" + totRow, "Total");
    var grand = 0;
    for (var q2 = 1; q2 <= 4; q2++) {
      var colLetter = cols[q2];
      var colSum = 0;
      for (var rr = 0; rr < rows.length; rr++) colSum += rows[rr][q2];
      grand += colSum;
      put(colLetter + totRow, "=SUM(" + colLetter + "2:" + colLetter + (totRow - 1) + ")", String(colSum));
    }
    put("F" + totRow, "=SUM(F2:F" + (totRow - 1) + ")", String(grand));
    // a friendly note
    put("A9", "Tip:");
    put("B9", "click a cell, type, press Enter.");
    return { name: "Sheet1", cells: cells };
  }

  // ---------- Toolbar actions ----------
  function doNew() {
    if (!TAURI) {
      // fresh empty demo
      loadSnapshot({ name: "Sheet1", cells: [] });
      statusMsg.textContent = "New workbook";
      return;
    }
    statusMsg.textContent = "Creating…";
    invoke("new_workbook").then(function (wb) {
      sheetIndex = (wb && typeof wb.active === "number") ? wb.active : 0;
      var s = wb && wb.sheets && wb.sheets[sheetIndex];
      loadSnapshot(s || { name: "Sheet1", cells: [] });
      statusMsg.textContent = "Ready";
    }).catch(function () { statusMsg.textContent = "Could not create"; });
  }

  function doOpen() {
    if (!TAURI) { statusMsg.textContent = "Open needs the desktop app"; return; }
    statusMsg.textContent = "Opening…";
    invoke("open_workbook").then(function (res) {
      if (res && res.snapshot) {
        loadSnapshot(res.snapshot);
        statusMsg.textContent = res.path ? ("Opened " + baseName(res.path)) : "Opened";
      } else {
        statusMsg.textContent = "Ready";
      }
    }).catch(function () { statusMsg.textContent = "Open failed"; });
  }

  function doSaveAs(format) {
    if (!TAURI) { statusMsg.textContent = "Save needs the desktop app"; return; }
    statusMsg.textContent = "Saving…";
    invoke("save_workbook", { format: format, path: null }).then(function (res) {
      if (res && res.path) statusMsg.textContent = "Saved " + baseName(res.path);
      else statusMsg.textContent = "Ready";
    }).catch(function () { statusMsg.textContent = "Save failed"; });
  }

  function baseName(p) {
    var s = String(p);
    var i = Math.max(s.lastIndexOf("/"), s.lastIndexOf("\\"));
    return i >= 0 ? s.slice(i + 1) : s;
  }

  // ---------- Save-as menu ----------
  var saveBtn = document.getElementById("btn-saveas");
  var saveMenu = document.getElementById("saveas-menu");
  function openMenu() { saveMenu.hidden = false; saveBtn.setAttribute("aria-expanded", "true"); }
  function closeMenu() { saveMenu.hidden = true; saveBtn.setAttribute("aria-expanded", "false"); }
  saveBtn.addEventListener("click", function (e) {
    e.stopPropagation();
    if (saveMenu.hidden) openMenu(); else closeMenu();
  });
  saveMenu.addEventListener("click", function (e) {
    var item = e.target.closest(".menu-item");
    if (!item) return;
    closeMenu();
    doSaveAs(item.getAttribute("data-format"));
  });
  document.addEventListener("click", function (e) {
    if (!saveMenu.hidden && !e.target.closest("#saveas-wrap")) closeMenu();
  });

  // toolbar buttons
  document.getElementById("btn-new").addEventListener("click", doNew);
  document.getElementById("btn-open").addEventListener("click", doOpen);

  // format buttons are visual-only toggles for v1
  ["btn-bold", "btn-italic"].forEach(function (id) {
    var b = document.getElementById(id);
    b.addEventListener("click", function () {
      var on = b.getAttribute("aria-pressed") === "true";
      b.setAttribute("aria-pressed", on ? "false" : "true");
    });
  });
  var alignBtns = ["btn-align-left", "btn-align-center", "btn-align-right"].map(function (id) { return document.getElementById(id); });
  alignBtns.forEach(function (b) {
    b.addEventListener("click", function () {
      alignBtns.forEach(function (o) { o.setAttribute("aria-pressed", "false"); });
      b.setAttribute("aria-pressed", "true");
    });
  });

  // ---------- Formula bar ----------
  fxInput.addEventListener("keydown", function (e) {
    if (e.key === "Enter") {
      e.preventDefault();
      commitCell(sel.col, sel.row, fxInput.value, "down");
      scroller.focus();
    } else if (e.key === "Escape") {
      e.preventDefault();
      refreshActiveInputBar();
      scroller.focus();
    } else if (e.key === "Tab") {
      e.preventDefault();
      commitCell(sel.col, sel.row, fxInput.value, e.shiftKey ? "left" : "right");
      scroller.focus();
    }
  });

  // ---------- Editor key handling ----------
  editor.addEventListener("keydown", function (e) {
    if (e.key === "Enter") {
      e.preventDefault();
      closeEditor(e.shiftKey ? "up" : "down");
    } else if (e.key === "Tab") {
      e.preventDefault();
      closeEditor(e.shiftKey ? "left" : "right");
    } else if (e.key === "Escape") {
      e.preventDefault();
      editing = false;
      editor.hidden = true;
      editor.value = "";
      scroller.focus();
      draw();
    }
    e.stopPropagation();
  });
  editor.addEventListener("blur", function () {
    // commit on blur (clicking elsewhere) without moving selection
    if (editing) closeEditor("stay");
  });

  // ---------- Grid pointer ----------
  scroller.addEventListener("mousedown", function (e) {
    var hit = hitCell(e.clientX, e.clientY);
    if (!hit) return;
    if (editing) closeEditor("stay");
    setSelection(hit.col, hit.row);
    scroller.focus();
  });
  scroller.addEventListener("dblclick", function (e) {
    var hit = hitCell(e.clientX, e.clientY);
    if (!hit) return;
    setSelection(hit.col, hit.row);
    openEditor(null);
  });

  // ---------- Keyboard navigation on the grid ----------
  scroller.addEventListener("keydown", function (e) {
    if (editing) return; // editor handles its own keys
    var k = e.key;
    if (k === "ArrowDown") { e.preventDefault(); setSelection(sel.col, sel.row + 1); }
    else if (k === "ArrowUp") { e.preventDefault(); setSelection(sel.col, sel.row - 1); }
    else if (k === "ArrowLeft") { e.preventDefault(); setSelection(sel.col - 1, sel.row); }
    else if (k === "ArrowRight") { e.preventDefault(); setSelection(sel.col + 1, sel.row); }
    else if (k === "Tab") { e.preventDefault(); setSelection(sel.col + (e.shiftKey ? -1 : 1), sel.row); }
    else if (k === "Enter") { e.preventDefault(); openEditor(null); }
    else if (k === "F2") { e.preventDefault(); openEditor(null); }
    else if (k === "Backspace" || k === "Delete") {
      e.preventDefault();
      commitCell(sel.col, sel.row, "", "stay");
    }
    else if (k === "Home") { e.preventDefault(); setSelection(0, sel.row); }
    else if (k === "PageDown") { e.preventDefault(); setSelection(sel.col, sel.row + pageRows()); }
    else if (k === "PageUp") { e.preventDefault(); setSelection(sel.col, sel.row - pageRows()); }
    else if (k.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey) {
      // printable char begins editing, replacing content
      e.preventDefault();
      openEditor(k);
    }
  });

  function pageRows() {
    return Math.max(1, Math.floor((viewport().h - HEAD_H) / ROW_H) - 1);
  }

  // ---------- Scroll sync ----------
  scroller.addEventListener("scroll", function () {
    scroll.x = scroller.scrollLeft;
    scroll.y = scroller.scrollTop;
    if (editing) {
      // keep editor glued to the active cell while scrolling
      editor.style.left = colX(sel.col) + "px";
      editor.style.top = rowY(sel.row) + "px";
    }
    draw();
  }, { passive: true });

  window.addEventListener("resize", resizeCanvas);

  // ---------- Boot ----------
  function boot() {
    resizeCanvas();
    if (TAURI) {
      statusMode.textContent = "Connected";
      invoke("new_workbook").then(function (wb) {
        sheetIndex = (wb && typeof wb.active === "number") ? wb.active : 0;
        var s = wb && wb.sheets && wb.sheets[sheetIndex];
        loadSnapshot(s || demoSnapshot());
        statusMsg.textContent = "Ready";
      }).catch(function () {
        loadSnapshot(demoSnapshot());
        statusMode.textContent = "Offline workbook";
        statusMsg.textContent = "Ready";
      });
    } else {
      statusMode.textContent = "Offline workbook";
      loadSnapshot(demoSnapshot());
      statusMsg.textContent = "Ready";
    }
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", boot);
  } else {
    boot();
  }
})();
