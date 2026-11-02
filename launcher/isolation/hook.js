// SPDX-License-Identifier: MIT
// Tauri's isolation hook: every IPC call the page makes passes through here before it reaches the backend. A call
// whose command or arguments the generated table doesn't describe is held forever, so the page's own deadline ends
// it and the backend never sees it. Plain ES5: the isolation frame loads it as is.
(function () {
  "use strict";

  var commands = window.__RR_COMMANDS__ || {};
  var settingsKeys = window.__RR_SETTINGS_KEYS__ || [];
  var has = Object.prototype.hasOwnProperty;
  var toString = Object.prototype.toString;
  var CHANNEL = /^__CHANNEL__:[0-9]{1,16}$/;
  var SETTING_TEXT_MAX = 16;

  var eventName = { kind: "text", max: 64 };
  var label = { label: { kind: "optionalText", max: 64 } };

  // The calls @tauri-apps/api makes for the page itself: events and the window controls
  var tauri = {
    "plugin:event|listen": { event: eventName, target: { kind: "any" }, handler: { kind: "number" } },
    "plugin:event|unlisten": { event: eventName, eventId: { kind: "number" } },
    "plugin:window|minimize": label,
    "plugin:window|toggle_maximize": label,
    "plugin:window|is_maximized": label,
    "plugin:window|start_dragging": label,
    "plugin:window|internal_toggle_maximize": label,
    "plugin:window|close": label
  };

  // One promise for every refusal: nothing is allocated per dropped call
  var never = new Promise(function () {});

  // Cross-realm safe: the payload may come from another frame
  function isPlainObject(value) {
    return typeof value === "object" && value !== null && toString.call(value) === "[object Object]";
  }

  function isText(value, max) {
    return typeof value === "string" && value.length <= max;
  }

  function isFiniteNumber(value) {
    return typeof value === "number" && isFinite(value);
  }

  function fitsFields(value, fields) {
    if (!isPlainObject(value)) return false;
    var keys = Object.keys(value);
    var i;
    for (i = 0; i < keys.length; i++) {
      if (!has.call(fields, keys[i])) return false;
    }
    for (var name in fields) {
      if (!has.call(fields, name)) continue;
      if (!fits(has.call(value, name) ? value[name] : undefined, fields[name])) return false;
    }
    return true;
  }

  function fitsSettingsPatch(value) {
    if (!isPlainObject(value)) return false;
    var keys = Object.keys(value);
    for (var i = 0; i < keys.length; i++) {
      if (settingsKeys.indexOf(keys[i]) < 0) return false;
      var setting = value[keys[i]];
      if (typeof setting === "string") {
        if (setting.length > SETTING_TEXT_MAX) return false;
      } else if (typeof setting !== "boolean" && !isFiniteNumber(setting)) {
        return false;
      }
    }
    return true;
  }

  function fits(value, shape) {
    if (!isPlainObject(shape)) return false;
    switch (shape.kind) {
      case "text":
        return isText(value, shape.max);
      case "optionalText":
        return value === undefined || value === null || isText(value, shape.max);
      case "flag":
        return typeof value === "boolean";
      case "index":
        return isFiniteNumber(value) && Math.floor(value) === value && value >= 0 && value <= shape.max;
      case "name":
        return typeof value === "string" && shape.names.indexOf(value) >= 0;
      case "number":
        return (
          isFiniteNumber(value) &&
          (shape.min === undefined || value >= shape.min) &&
          (shape.max === undefined || value <= shape.max)
        );
      case "object":
        if (value === null) return shape.nullable === true;
        return fitsFields(value, shape.fields);
      case "settingsPatch":
        return fitsSettingsPatch(value);
      case "channel":
        return typeof value === "string" && CHANNEL.test(value);
      case "any":
        return true;
      default:
        return false;
    }
  }

  // Own properties only, so __proto__, constructor and the like are unknown commands
  function entryFor(cmd) {
    if (has.call(commands, cmd)) return commands[cmd];
    if (has.call(tauri, cmd)) return tauri[cmd];
    return null;
  }

  function allowed(payload) {
    if (typeof payload !== "object" || payload === null) return false;
    if (typeof payload.cmd !== "string") return false;
    var entry = entryFor(payload.cmd);
    if (!isPlainObject(entry)) return false;
    var args = payload.payload === undefined || payload.payload === null ? {} : payload.payload;
    return fitsFields(args, entry);
  }

  window.__TAURI_ISOLATION_HOOK__ = function (payload) {
    var ok = false;
    try {
      ok = allowed(payload);
    } catch (error) {
      ok = false;
    }
    return ok ? payload : never;
  };
})();
