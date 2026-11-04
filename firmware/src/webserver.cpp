#include "webserver.h"

#include <Arduino.h>
#include <WiFi.h>
#include <esp_camera.h>
#include <esp_http_server.h>

#include "camera.h"
#include "network.h"
#include "pins.h"
#include "settings.h"

static httpd_handle_t apiServer = nullptr;
static httpd_handle_t streamServer = nullptr;
static bool restartRequested = false;
static uint32_t restartAt = 0;

static const size_t MAX_FORM_BYTES = 512;
static const char STREAM_TYPE[] = "multipart/x-mixed-replace;boundary=frame";

static const char SETUP_PAGE[] PROGMEM = R"HTML(<!doctype html><meta name=viewport content="width=device-width,initial-scale=1"><title>Camera setup</title>
<style>body{font:16px system-ui;max-width:420px;margin:24px auto;padding:0 16px}input,select,button{width:100%;padding:11px;margin:6px 0 14px;font:inherit;box-sizing:border-box}button{background:#2f6fed;color:#fff;border:0;border-radius:8px}small{color:#667}</style>
<h2>Camera setup</h2><form method=post action=/save>
<label>Wifi network (2.4 GHz)</label><select id=n onchange="s.value=this.value"></select>
<input id=s name=ssid placeholder="or type its name" required>
<label>Wifi password</label><input name=pass type=password autocomplete=off>
<label>Access token</label><input id=t name=token required minlength=12 maxlength=64 pattern="[A-Za-z0-9_-]+">
<label>Current token</label><input name=current placeholder="only if the camera was already set up">
<small>The token is what the server uses to talk to this camera. Copy it, you will need it once.</small>
<button>Save and restart</button></form>
<script>
fetch('/scan').then(r=>r.json()).then(l=>{n.innerHTML='<option value="">Choose a network</option>'+l.sort((a,b)=>b.rssi-a.rssi).map(x=>'<option>'+x.ssid.replace(/</g,'&lt;')+'</option>').join('')});
t.value=[...crypto.getRandomValues(new Uint8Array(12))].map(b=>b.toString(16).padStart(2,'0')).join('');
</script>)HTML";

static bool tokenMatches(const char *given) {
  const String &expected = settings.token;
  if (expected.isEmpty()) return false;
  size_t length = strlen(given);
  uint8_t difference = length != expected.length();
  for (size_t i = 0; i < expected.length(); i++) difference |= (i < length ? given[i] : 0) ^ expected[i];
  return difference == 0;
}

static bool tokenIsPlain(const String &token) {
  for (size_t i = 0; i < token.length(); i++) {
    char c = token[i];
    if (!isalnum((unsigned char)c) && c != '-' && c != '_') return false;
  }
  return true;
}

static bool authorized(httpd_req_t *req) {
  char header[160];
  if (httpd_req_get_hdr_value_str(req, "Authorization", header, sizeof(header)) != ESP_OK) return false;
  return strncmp(header, "Bearer ", 7) == 0 && tokenMatches(header + 7);
}

static esp_err_t reply(httpd_req_t *req, const char *status, const char *type, const char *body) {
  httpd_resp_set_status(req, status);
  httpd_resp_set_type(req, type);
  return httpd_resp_send(req, body, HTTPD_RESP_USE_STRLEN);
}

static esp_err_t deny(httpd_req_t *req) { return reply(req, "401 Unauthorized", "text/plain", "unauthorized"); }
static esp_err_t standby(httpd_req_t *req) { return reply(req, "503 Service Unavailable", "text/plain", "standby"); }

static esp_err_t healthHandler(httpd_req_t *req) { return reply(req, "200 OK", "text/plain", "ok"); }

static esp_err_t captureHandler(httpd_req_t *req) {
  if (!authorized(req)) return deny(req);
  if (!cameraRunning()) return standby(req);
  camera_fb_t *frame = cameraFrame();
  cameraNoteFrame(frame != nullptr);
  if (!frame) return reply(req, "500 Internal Server Error", "text/plain", "no frame");
  httpd_resp_set_type(req, "image/jpeg");
  httpd_resp_set_hdr(req, "Cache-Control", "no-store");
  esp_err_t result = httpd_resp_send(req, (const char *)frame->buf, frame->len);
  cameraRelease(frame);
  return result;
}

static esp_err_t statusHandler(httpd_req_t *req) {
  if (!authorized(req)) return deny(req);
  int framesize, quality, vflip, hmirror;
  cameraDescribe(framesize, quality, vflip, hmirror);
  char body[420];
  snprintf(body, sizeof(body),
           "{\"fw\":\"%s\",\"hostname\":\"%s\",\"uptime_s\":%lu,\"rssi\":%d,\"ip\":\"%s\",\"mac\":\"%s\","
           "\"free_heap\":%u,\"free_psram\":%u,\"framesize\":%d,\"quality\":%d,\"vflip\":%d,\"hmirror\":%d,"
           "\"standby\":%d,\"reset_reason\":%d}",
           FW_VERSION, settings.hostname.c_str(), millis() / 1000, WiFi.RSSI(), WiFi.localIP().toString().c_str(),
           WiFi.macAddress().c_str(), ESP.getFreeHeap(), ESP.getFreePsram(), framesize, quality, vflip, hmirror,
           cameraRunning() ? 0 : 1, (int)esp_reset_reason());
  return reply(req, "200 OK", "application/json", body);
}

static esp_err_t controlHandler(httpd_req_t *req) {
  if (!authorized(req)) return deny(req);
  char query[96], name[24], value[16];
  if (httpd_req_get_url_query_str(req, query, sizeof(query)) != ESP_OK ||
      httpd_query_key_value(query, "var", name, sizeof(name)) != ESP_OK ||
      httpd_query_key_value(query, "val", value, sizeof(value)) != ESP_OK) {
    return reply(req, "400 Bad Request", "text/plain", "var and val are required");
  }
  int number = atoi(value);
  if (strcmp(name, "standby") == 0) {
    if (number) cameraStop();
    else if (!cameraStart()) return reply(req, "500 Internal Server Error", "text/plain", "camera did not start");
    return reply(req, "200 OK", "text/plain", "ok");
  }
  if (strcmp(name, "led") == 0) {
    digitalWrite(FLASH_LED_GPIO, number ? HIGH : LOW);
    return reply(req, "200 OK", "text/plain", "ok");
  }
  if (!cameraRunning()) return standby(req);
  if (!cameraControl(name, number)) return reply(req, "400 Bad Request", "text/plain", "unknown or invalid setting");
  return reply(req, "200 OK", "text/plain", "ok");
}

static esp_err_t streamHandler(httpd_req_t *req) {
  if (!authorized(req)) return deny(req);
  if (!cameraRunning()) return standby(req);
  httpd_resp_set_type(req, STREAM_TYPE);
  httpd_resp_set_hdr(req, "Cache-Control", "no-store");

  esp_err_t result = ESP_OK;
  while (result == ESP_OK && cameraRunning()) {
    camera_fb_t *frame = cameraFrame();
    cameraNoteFrame(frame != nullptr);
    if (!frame) return ESP_FAIL;
    char head[96];
    size_t headLength = snprintf(head, sizeof(head), "\r\n--frame\r\nContent-Type: image/jpeg\r\nContent-Length: %u\r\n\r\n", (unsigned)frame->len);
    result = httpd_resp_send_chunk(req, head, headLength);
    if (result == ESP_OK) result = httpd_resp_send_chunk(req, (const char *)frame->buf, frame->len);
    cameraRelease(frame);
  }
  return result;
}

static String formValue(const String &body, const char *key) {
  String prefix = String(key) + "=";
  int start = body.startsWith(prefix) ? 0 : body.indexOf("&" + prefix);
  if (start < 0) return "";
  start += body.startsWith(prefix) ? prefix.length() : prefix.length() + 1;
  int end = body.indexOf('&', start);
  String raw = body.substring(start, end < 0 ? body.length() : end);
  String decoded;
  for (size_t i = 0; i < raw.length(); i++) {
    char c = raw[i];
    if (c == '+') decoded += ' ';
    else if (c == '%' && i + 2 < raw.length()) {
      decoded += (char)strtol(raw.substring(i + 1, i + 3).c_str(), nullptr, 16);
      i += 2;
    } else decoded += c;
  }
  return decoded;
}

static esp_err_t setupPageHandler(httpd_req_t *req) {
  if (!networkPortalActive()) return reply(req, "200 OK", "text/plain", "camd camera");
  return reply(req, "200 OK", "text/html", SETUP_PAGE);
}

static esp_err_t scanHandler(httpd_req_t *req) {
  if (!networkPortalActive()) return deny(req);
  return reply(req, "200 OK", "application/json", networkScanJson().c_str());
}

static esp_err_t saveHandler(httpd_req_t *req) {
  if (!networkPortalActive()) return deny(req);
  if (req->content_len == 0 || req->content_len > MAX_FORM_BYTES) return reply(req, "413 Payload Too Large", "text/plain", "form too large");
  char buffer[MAX_FORM_BYTES + 1];
  size_t received = 0;
  while (received < req->content_len) {
    int chunk = httpd_req_recv(req, buffer + received, req->content_len - received);
    if (chunk <= 0) return reply(req, "400 Bad Request", "text/plain", "incomplete form");
    received += chunk;
  }
  buffer[received] = 0;
  String body(buffer);
  String ssid = formValue(body, "ssid"), token = formValue(body, "token");
  if (settings.wifiConfigured() && !tokenMatches(formValue(body, "current").c_str())) return deny(req);
  if (ssid.isEmpty() || token.length() < 12 || token.length() > 64 || !tokenIsPlain(token)) {
    return reply(req, "400 Bad Request", "text/plain", "a network and a token of 12 to 64 letters, digits, - or _ are required");
  }
  settings.save("ssid", ssid);
  settings.save("pass", formValue(body, "pass"));
  settings.save("token", token);
  restartRequested = true;
  restartAt = millis() + 2000;
  Serial.printf("setup saved for '%s'\n", ssid.c_str());
  String page = "<!doctype html><meta name=viewport content='width=device-width,initial-scale=1'><body style='font:16px system-ui;max-width:420px;margin:24px auto;padding:0 16px'><h2>Saved</h2><p>The camera is restarting and joining your wifi. Put this token in the server as <b>CAMD_CAMERA_TOKEN</b>:</p><p><code style='word-break:break-all'>" + token + "</code></p><p>Then set <b>camera_host</b> to <b>" + settings.hostname + ".local</b> or to the IP your router gives it.</p>";
  return reply(req, "200 OK", "text/html", page.c_str());
}

static esp_err_t redirectHandler(httpd_req_t *req, httpd_err_code_t) {
  if (!networkPortalActive()) return reply(req, "404 Not Found", "text/plain", "not found");
  httpd_resp_set_status(req, "302 Found");
  httpd_resp_set_hdr(req, "Location", "http://192.168.4.1/");
  return httpd_resp_send(req, nullptr, 0);
}

static void route(httpd_handle_t server, const char *path, httpd_method_t method, esp_err_t (*handler)(httpd_req_t *)) {
  httpd_uri_t uri = {path, method, handler, nullptr};
  httpd_register_uri_handler(server, &uri);
}

void webserverStart() {
  httpd_config_t config = HTTPD_DEFAULT_CONFIG();
  config.server_port = 80;
  config.max_uri_handlers = 10;
  config.lru_purge_enable = true;
  if (httpd_start(&apiServer, &config) == ESP_OK) {
    route(apiServer, "/", HTTP_GET, setupPageHandler);
    route(apiServer, "/health", HTTP_GET, healthHandler);
    route(apiServer, "/capture", HTTP_GET, captureHandler);
    route(apiServer, "/status", HTTP_GET, statusHandler);
    route(apiServer, "/control", HTTP_GET, controlHandler);
    route(apiServer, "/scan", HTTP_GET, scanHandler);
    route(apiServer, "/save", HTTP_POST, saveHandler);
    httpd_register_err_handler(apiServer, HTTPD_404_NOT_FOUND, redirectHandler);
  }

  config.server_port = 81;
  config.ctrl_port += 1;
  if (httpd_start(&streamServer, &config) == ESP_OK) route(streamServer, "/stream", HTTP_GET, streamHandler);
}

void webserverLoop() {
  if (restartRequested && millis() > restartAt) ESP.restart();
}
