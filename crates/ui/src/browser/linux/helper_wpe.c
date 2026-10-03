// WPE WebKit runs in its own process. Only rendered pixels and explicit browser
// commands cross the pipe; GPUI owns all visible windows and input routing.
//
// The helper is WebKit's display: it defines the screen, hands each page a
// view, receives every rendered buffer, and supplies the vsync tick. WebKit
// paces itself by that tick, so pages animate at the host's rate rather than
// a toolkit's fixed 60 Hz.
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <glib-unix.h>
#include <json-glib/json-glib.h>
#include <signal.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <wpe/webkit.h>
#include <wpe/wpe-platform.h>
#include <xkbcommon/xkbcommon.h>

#define MAX_COMMAND (1024 * 1024)
#define MAX_DIMENSION 8192
#define MAX_HTML (32 * 1024 * 1024)
#define DEFAULT_REFRESH_HZ 144

typedef struct {
    guint id;
    WebKitWebView *web;
    WPEView *view;
    WPEToplevel *toplevel;
    gboolean visible;
    gchar *error;
    guint width, height;
    double scale;
    WebKitOptionMenu *options;
    gchar *context_link;
    // Where the pointer last went down, for placing the context menu.
    double press_x, press_y;
    // A workspace document is loaded from text and commits as about:blank.
    GString *html;
    gboolean local;
} Page;
static GHashTable *pages;
static WebKitNetworkSession *session;
static WPEDisplay *display;
static WPEScreen *screen;
static WPEKeymap *keymap;
static int refresh_mhz;
static GByteArray *input;
static GMainLoop *main_loop;

static gboolean write_all(const void *data, size_t length) {
    const char *p = data;
    while (length) {
        ssize_t n = write(STDOUT_FILENO, p, length);
        if (n < 0 && errno == EINTR)
            continue;
        if (n <= 0) {
            g_main_loop_quit(main_loop);
            return FALSE;
        }
        p += n;
        length -= n;
    }
    return TRUE;
}
static void send_packet(char kind, guint id, const void *data, guint length) {
    guint32 header[2] = {GUINT32_TO_LE(id), GUINT32_TO_LE(length)};
    if (!write_all(&kind, 1) || !write_all(header, sizeof header))
        return;
    write_all(data, length);
}
static void send_json(char kind, guint id, JsonBuilder *builder) {
    JsonGenerator *gen = json_generator_new();
    JsonNode *root = json_builder_get_root(builder);
    json_generator_set_root(gen, root);
    gsize length;
    gchar *data = json_generator_to_data(gen, &length);
    send_packet(kind, id, data, length);
    g_free(data);
    json_node_free(root);
    g_object_unref(gen);
    g_object_unref(builder);
}
static void member_string(JsonBuilder *b, const char *name, const char *value) {
    json_builder_set_member_name(b, name);
    if (value)
        json_builder_add_string_value(b, value);
    else
        json_builder_add_null_value(b);
}
static void member_bool(JsonBuilder *b, const char *name, gboolean value) {
    json_builder_set_member_name(b, name);
    json_builder_add_boolean_value(b, value);
}
// Relay the platform IME through GPUI while WebKit retains editor semantics.
typedef struct {
    WebKitInputMethodContext parent;
    guint id;
    gchar *preedit;
    guint cursor;
} BrowserIM;
typedef struct {
    WebKitInputMethodContextClass parent;
} BrowserIMClass;
G_DEFINE_TYPE(BrowserIM, browser_im, WEBKIT_TYPE_INPUT_METHOD_CONTEXT)
static void im_preedit(WebKitInputMethodContext *ctx, gchar **text, GList **underlines,
                       guint *cursor) {
    BrowserIM *im = (BrowserIM *)ctx;
    *text = g_strdup(im->preedit ?: "");
    *cursor = im->cursor;
    *underlines =
        im->preedit && *im->preedit
            ? g_list_append(NULL,
                            webkit_input_method_underline_new(0, g_utf8_strlen(im->preedit, -1)))
            : NULL;
}
static gboolean im_filter(WebKitInputMethodContext *ctx, gpointer event) { return FALSE; }
static void im_focus(WebKitInputMethodContext *ctx, gboolean focused) {
    JsonBuilder *b = json_builder_new();
    json_builder_begin_object(b);
    member_bool(b, "focused", focused);
    json_builder_end_object(b);
    send_json('I', ((BrowserIM *)ctx)->id, b);
}
static void im_focus_in(WebKitInputMethodContext *ctx) { im_focus(ctx, TRUE); }
static void im_focus_out(WebKitInputMethodContext *ctx) { im_focus(ctx, FALSE); }
static void im_cursor(WebKitInputMethodContext *ctx, int x, int y, int w, int h) {
    JsonBuilder *b = json_builder_new();
    json_builder_begin_object(b);
    json_builder_set_member_name(b, "caret");
    json_builder_begin_array(b);
    json_builder_add_int_value(b, x);
    json_builder_add_int_value(b, y);
    json_builder_add_int_value(b, w);
    json_builder_add_int_value(b, h);
    json_builder_end_array(b);
    json_builder_end_object(b);
    send_json('I', ((BrowserIM *)ctx)->id, b);
}
static void im_surrounding(WebKitInputMethodContext *ctx, const gchar *text, guint length,
                           guint cursor, guint selection) {
    // Password contents must never leave WebKit's editor process boundary.
    if (webkit_input_method_context_get_input_purpose(ctx) == WEBKIT_INPUT_PURPOSE_PASSWORD)
        return;
    gchar *copy = g_strndup(text, length);
    JsonBuilder *b = json_builder_new();
    json_builder_begin_object(b);
    member_string(b, "text", copy);
    member_bool(b, "focused", TRUE);
    g_free(copy);
    json_builder_set_member_name(b, "cursor");
    json_builder_add_int_value(b, cursor);
    json_builder_set_member_name(b, "selection");
    json_builder_add_int_value(b, selection);
    json_builder_end_object(b);
    send_json('I', ((BrowserIM *)ctx)->id, b);
}
static void im_finalize(GObject *object) {
    g_free(((BrowserIM *)object)->preedit);
    G_OBJECT_CLASS(browser_im_parent_class)->finalize(object);
}
static void browser_im_class_init(BrowserIMClass *klass) {
    G_OBJECT_CLASS(klass)->finalize = im_finalize;
    WebKitInputMethodContextClass *im = WEBKIT_INPUT_METHOD_CONTEXT_CLASS(klass);
    im->get_preedit = im_preedit;
    im->filter_key_event = im_filter;
    im->notify_focus_in = im_focus_in;
    im->notify_focus_out = im_focus_out;
    im->notify_cursor_area = im_cursor;
    im->notify_surrounding = im_surrounding;
}
static void browser_im_init(BrowserIM *im) { im->preedit = g_strdup(""); }
static void state(Page *p) {
    JsonBuilder *b = json_builder_new();
    json_builder_begin_object(b);
    member_string(b, "url", webkit_web_view_get_uri(p->web));
    member_string(b, "title", webkit_web_view_get_title(p->web) ?: "");
    member_string(b, "error", p->error);
    member_bool(b, "loading", webkit_web_view_is_loading(p->web));
    member_bool(b, "can_back", webkit_web_view_can_go_back(p->web));
    member_bool(b, "can_forward", webkit_web_view_can_go_forward(p->web));
    json_builder_end_object(b);
    send_json('S', p->id, b);
}
static void changed(GObject *web, GParamSpec *spec, Page *p) { state(p); }
static void loaded(WebKitWebView *web, WebKitLoadEvent event, Page *p) {
    if (event == WEBKIT_LOAD_STARTED)
        g_clear_pointer(&p->error, g_free);
    state(p);
}
static gboolean failed(WebKitWebView *web, WebKitLoadEvent event, const char *uri, GError *error,
                       Page *p) {
    if (g_error_matches(error, WEBKIT_NETWORK_ERROR, WEBKIT_NETWORK_ERROR_CANCELLED))
        return TRUE;
    g_free(p->error);
    p->error = g_strdup(error->message);
    state(p);
    return TRUE;
}
static void terminated(WebKitWebView *web, WebKitWebProcessTerminationReason reason, Page *p) {
    g_free(p->error);
    p->error = g_strdup("The browser process stopped. Reload to try again.");
    state(p);
}
static gboolean allowed(const char *uri) {
    GUri *u = g_uri_parse(uri, G_URI_FLAGS_NONE, NULL);
    if (!u)
        return FALSE;
    const char *scheme = g_uri_get_scheme(u);
    gboolean ok = scheme && (!strcmp(scheme, "http") || !strcmp(scheme, "https")) &&
                  g_uri_get_host(u) && !g_uri_get_userinfo(u);
    g_uri_unref(u);
    return ok;
}
static gboolean policy(WebKitWebView *web, WebKitPolicyDecision *decision,
                       WebKitPolicyDecisionType type, Page *p) {
    if (type == WEBKIT_POLICY_DECISION_TYPE_NAVIGATION_ACTION ||
        type == WEBKIT_POLICY_DECISION_TYPE_NEW_WINDOW_ACTION) {
        WebKitNavigationAction *action = webkit_navigation_policy_decision_get_navigation_action(
            WEBKIT_NAVIGATION_POLICY_DECISION(decision));
        const char *uri = webkit_uri_request_get_uri(webkit_navigation_action_get_request(action));
        // A document's own anchors navigate within its blank address.
        if (type == WEBKIT_POLICY_DECISION_TYPE_NAVIGATION_ACTION && p->local && uri &&
            (!strcmp(uri, "about:blank") || g_str_has_prefix(uri, "about:blank#")))
            return FALSE;
        if (!allowed(uri)) {
            webkit_policy_decision_ignore(decision);
            return TRUE;
        }
        if (type == WEBKIT_POLICY_DECISION_TYPE_NEW_WINDOW_ACTION) {
            if (webkit_navigation_action_is_user_gesture(action))
                send_packet('N', p->id, uri, strlen(uri));
            webkit_policy_decision_ignore(decision);
            return TRUE;
        }
    }
    return FALSE;
}
static gboolean permission(WebKitWebView *web, WebKitPermissionRequest *request, Page *p) {
    webkit_permission_request_deny(request);
    return TRUE;
}
static JsonBuilder *menu_builder(double x, double y) {
    JsonBuilder *b = json_builder_new();
    json_builder_begin_object(b);
    json_builder_set_member_name(b, "x");
    json_builder_add_double_value(b, x);
    json_builder_set_member_name(b, "y");
    json_builder_add_double_value(b, y);
    json_builder_set_member_name(b, "items");
    json_builder_begin_array(b);
    return b;
}
static void menu_item(JsonBuilder *b, const char *label, const char *action, gboolean enabled,
                      gboolean selected) {
    json_builder_begin_object(b);
    member_string(b, "label", label);
    member_string(b, "action", action);
    member_bool(b, "enabled", enabled);
    member_bool(b, "selected", selected);
    json_builder_end_object(b);
}
static void send_menu(Page *p, JsonBuilder *b) {
    json_builder_end_array(b);
    json_builder_end_object(b);
    send_json('M', p->id, b);
}
static gboolean context_menu(WebKitWebView *web, WebKitContextMenu *menu, WebKitHitTestResult *hit,
                             Page *p) {
    double x = p->press_x, y = p->press_y;
    g_free(p->context_link);
    p->context_link = NULL;
    JsonBuilder *b = menu_builder(x / p->scale, y / p->scale);
    if (webkit_hit_test_result_context_is_link(hit)) {
        p->context_link = g_strdup(webkit_hit_test_result_get_link_uri(hit));
        menu_item(b, "Open link in new tab", "open-link", allowed(p->context_link), FALSE);
        menu_item(b, "Copy link address", "copy-link", TRUE, FALSE);
    }
    menu_item(b, "Copy", "copy",
              webkit_hit_test_result_context_is_selection(hit) ||
                  webkit_hit_test_result_context_is_editable(hit),
              FALSE);
    if (webkit_hit_test_result_context_is_editable(hit))
        menu_item(b, "Paste", "text", TRUE, FALSE);
    menu_item(b, "Select all", "select-all", TRUE, FALSE);
    menu_item(b, "Back", "back", webkit_web_view_can_go_back(web), FALSE);
    menu_item(b, "Forward", "forward", webkit_web_view_can_go_forward(web), FALSE);
    menu_item(b, "Reload", "reload", TRUE, FALSE);
    send_menu(p, b);
    return TRUE;
}
static gboolean option_menu(WebKitWebView *web, WebKitOptionMenu *menu, WebKitRectangle *rect,
                            Page *p) {
    g_set_object(&p->options, menu);
    JsonBuilder *b = menu_builder(rect->x / p->scale, (rect->y + rect->height) / p->scale);
    for (guint i = 0; i < webkit_option_menu_get_n_items(menu); i++) {
        WebKitOptionMenuItem *item = webkit_option_menu_get_item(menu, i);
        gchar *action = g_strdup_printf("option:%u", i);
        menu_item(b, webkit_option_menu_item_get_label(item), action,
                  webkit_option_menu_item_is_enabled(item) &&
                      !webkit_option_menu_item_is_group_label(item),
                  webkit_option_menu_item_is_selected(item));
        g_free(action);
    }
    send_menu(p, b);
    return TRUE;
}
static void download(WebKitNetworkSession *s, WebKitDownload *item, gpointer data) {
    webkit_download_cancel(item);
}

// ---- vsync. A ticking thread stands in for the display's vertical sync.
G_DECLARE_FINAL_TYPE(BrowserSync, browser_sync, BROWSER, SYNC, WPEScreenSyncObserver)
struct _BrowserSync {
    WPEScreenSyncObserver parent;
    GThread *thread;
    gint running;
};
G_DEFINE_TYPE(BrowserSync, browser_sync, WPE_TYPE_SCREEN_SYNC_OBSERVER)
static gpointer sync_thread(gpointer data) {
    BrowserSync *s = data;
    gint64 period = G_GINT64_CONSTANT(1000000000) / refresh_mhz, next = g_get_monotonic_time();
    while (g_atomic_int_get(&s->running)) {
        WPE_SCREEN_SYNC_OBSERVER_GET_CLASS(s)->sync(WPE_SCREEN_SYNC_OBSERVER(s));
        next += period;
        gint64 now = g_get_monotonic_time();
        if (next > now)
            g_usleep(next - now);
        else
            next = now;
    }
    g_object_unref(s);
    return NULL;
}
static void sync_start(WPEScreenSyncObserver *observer) {
    BrowserSync *s = BROWSER_SYNC(observer);
    if (g_atomic_int_compare_and_exchange(&s->running, 0, 1))
        s->thread = g_thread_new("browser-vsync", sync_thread, g_object_ref(s));
}
static void sync_stop(WPEScreenSyncObserver *observer) {
    BrowserSync *s = BROWSER_SYNC(observer);
    // WebKit may stop the observer from inside a tick, on the ticking thread.
    if (g_atomic_int_compare_and_exchange(&s->running, 1, 0)) {
        if (s->thread == g_thread_self())
            g_thread_unref(s->thread);
        else
            g_thread_join(s->thread);
        s->thread = NULL;
    }
}
static void browser_sync_class_init(BrowserSyncClass *klass) {
    WPE_SCREEN_SYNC_OBSERVER_CLASS(klass)->start = sync_start;
    WPE_SCREEN_SYNC_OBSERVER_CLASS(klass)->stop = sync_stop;
}
static void browser_sync_init(BrowserSync *s) {}

// ---- screen
G_DECLARE_FINAL_TYPE(BrowserScreen, browser_screen, BROWSER, SCREEN, WPEScreen)
struct _BrowserScreen {
    WPEScreen parent;
};
G_DEFINE_TYPE(BrowserScreen, browser_screen, WPE_TYPE_SCREEN)
static WPEScreenSyncObserver *screen_sync(WPEScreen *s) {
    static WPEScreenSyncObserver *observer;
    if (!observer)
        observer = g_object_new(browser_sync_get_type(), NULL);
    return observer;
}
static void browser_screen_class_init(BrowserScreenClass *klass) {
    WPE_SCREEN_CLASS(klass)->get_sync_observer = screen_sync;
}
static void browser_screen_init(BrowserScreen *s) {}

// ---- toplevel. One per page: it carries the size and the active state.
G_DECLARE_FINAL_TYPE(BrowserToplevel, browser_toplevel, BROWSER, TOPLEVEL, WPEToplevel)
struct _BrowserToplevel {
    WPEToplevel parent;
};
G_DEFINE_TYPE(BrowserToplevel, browser_toplevel, WPE_TYPE_TOPLEVEL)
static WPEScreen *toplevel_screen(WPEToplevel *t) { return screen; }
static gboolean toplevel_resize(WPEToplevel *t, int width, int height) {
    wpe_toplevel_resized(t, width, height);
    return TRUE;
}
static void browser_toplevel_class_init(BrowserToplevelClass *klass) {
    WPE_TOPLEVEL_CLASS(klass)->get_screen = toplevel_screen;
    WPE_TOPLEVEL_CLASS(klass)->resize = toplevel_resize;
}
static void browser_toplevel_init(BrowserToplevel *t) {}

// ---- view. WebKit hands over each finished buffer and waits to be told it
// was presented; that acknowledgement is what lets the next frame start.
G_DECLARE_FINAL_TYPE(BrowserView, browser_view, BROWSER, VIEW, WPEView)
struct _BrowserView {
    WPEView parent;
    Page *page;
};
G_DEFINE_TYPE(BrowserView, browser_view, WPE_TYPE_VIEW)
typedef struct {
    WPEView *view;
    WPEBuffer *buffer;
} Presented;
static gboolean presented(gpointer data) {
    Presented *done = data;
    wpe_view_buffer_rendered(done->view, done->buffer);
    wpe_view_buffer_released(done->view, done->buffer);
    g_object_unref(done->buffer);
    g_object_unref(done->view);
    g_free(done);
    return G_SOURCE_REMOVE;
}
static void send_frame(Page *p, WPEBuffer *buffer) {
    guint w = wpe_buffer_get_width(buffer), h = wpe_buffer_get_height(buffer);
    // A buffer from before a resize is acknowledged but never shown.
    if (!p->visible || w != p->width || h != p->height || w > MAX_DIMENSION || h > MAX_DIMENSION ||
        G_BYTE_ORDER != G_LITTLE_ENDIAN)
        return;
    GBytes *bytes = wpe_buffer_import_to_pixels(buffer, NULL);
    if (!bytes)
        return;
    gsize size;
    const guchar *src = g_bytes_get_data(bytes, &size);
    gsize stride = WPE_IS_BUFFER_SHM(buffer) ? wpe_buffer_shm_get_stride(WPE_BUFFER_SHM(buffer))
                                             : (gsize)w * 4;
    if (stride < (gsize)w * 4 || size < stride * (h - 1) + (gsize)w * 4)
        return;
    guint length = 12 + w * h * 4;
    // One buffer serves every frame: a fresh megabyte-scale allocation per
    // frame would spend its time faulting pages in.
    static guchar *out;
    static guint capacity;
    if (capacity < length) {
        out = g_realloc(out, length);
        capacity = length;
    }
    ((guint32 *)out)[0] = GUINT32_TO_LE(w);
    ((guint32 *)out)[1] = GUINT32_TO_LE(h);
    float scale = p->scale;
    guint32 scale_bits;
    memcpy(&scale_bits, &scale, 4);
    ((guint32 *)out)[2] = GUINT32_TO_LE(scale_bits);
    // WebKit's native-endian ARGB is already the BGRA byte order of GPUI's
    // image atlas. The page is opaque; its alpha byte carries nothing.
    for (guint y = 0; y < h; y++) {
        const guint32 *from = (const guint32 *)(src + (gsize)y * stride);
        guint32 *to = (guint32 *)(out + 12 + (gsize)y * w * 4);
        for (guint x = 0; x < w; x++)
            to[x] = from[x] | 0xff000000u;
    }
    send_packet('F', p->id, out, length);
}
static gboolean render_buffer(WPEView *view, WPEBuffer *buffer, const WPERectangle *damage,
                              guint n_damage, GError **error) {
    Page *p = BROWSER_VIEW(view)->page;
    if (p)
        send_frame(p, buffer);
    Presented *done = g_new(Presented, 1);
    done->view = g_object_ref(view);
    done->buffer = g_object_ref(buffer);
    g_idle_add(presented, done);
    return TRUE;
}
static gboolean view_can_be_mapped(WPEView *view) { return TRUE; }
static void browser_view_class_init(BrowserViewClass *klass) {
    WPE_VIEW_CLASS(klass)->render_buffer = render_buffer;
    WPE_VIEW_CLASS(klass)->can_be_mapped = view_can_be_mapped;
}
static void browser_view_init(BrowserView *v) {}

// ---- display
G_DECLARE_FINAL_TYPE(BrowserDisplay, browser_display, BROWSER, DISPLAY, WPEDisplay)
struct _BrowserDisplay {
    WPEDisplay parent;
};
G_DEFINE_TYPE(BrowserDisplay, browser_display, WPE_TYPE_DISPLAY)
static gboolean display_connect(WPEDisplay *d, GError **error) { return TRUE; }
static WPEView *display_create_view(WPEDisplay *d) {
    return g_object_new(browser_view_get_type(), "display", d, NULL);
}
static guint display_n_screens(WPEDisplay *d) { return 1; }
static WPEScreen *display_screen(WPEDisplay *d, guint index) { return index ? NULL : screen; }
static WPEKeymap *display_keymap(WPEDisplay *d) { return keymap; }
static void browser_display_class_init(BrowserDisplayClass *klass) {
    WPEDisplayClass *display_class = WPE_DISPLAY_CLASS(klass);
    display_class->connect = display_connect;
    display_class->create_view = display_create_view;
    display_class->get_n_screens = display_n_screens;
    display_class->get_screen = display_screen;
    display_class->get_keymap = display_keymap;
}
static void browser_display_init(BrowserDisplay *d) {}

static void free_page(gpointer data) {
    Page *p = data;
    if (p->options) {
        webkit_option_menu_close(p->options);
        g_clear_object(&p->options);
    }
    // Buffers still in flight must not reach a page that no longer exists.
    BROWSER_VIEW(p->view)->page = NULL;
    g_signal_handlers_disconnect_by_data(p->web, p);
    g_object_unref(p->web);
    g_object_unref(p->toplevel);
    g_free(p->error);
    g_free(p->context_link);
    if (p->html)
        g_string_free(p->html, TRUE);
    g_free(p);
}
static Page *new_page(guint id) {
    Page *p = g_new0(Page, 1);
    p->id = id;
    p->width = 800;
    p->height = 600;
    p->scale = 1;
    p->visible = TRUE;
    p->web = g_object_new(WEBKIT_TYPE_WEB_VIEW, "display", display, "network-session", session,
                          NULL);
    p->view = webkit_web_view_get_wpe_view(p->web);
    BROWSER_VIEW(p->view)->page = p;
    p->toplevel = g_object_new(browser_toplevel_get_type(), "display", display, NULL);
    wpe_view_set_toplevel(p->view, p->toplevel);
    wpe_toplevel_resized(p->toplevel, p->width, p->height);
    wpe_toplevel_state_changed(p->toplevel, WPE_TOPLEVEL_STATE_ACTIVE);
    wpe_view_resized(p->view, p->width, p->height);
    wpe_view_set_visible(p->view, TRUE);
    wpe_view_map(p->view);
    wpe_view_focus_in(p->view);
    BrowserIM *im = g_object_new(browser_im_get_type(), NULL);
    im->id = id;
    webkit_web_view_set_input_method_context(p->web, WEBKIT_INPUT_METHOD_CONTEXT(im));
    g_object_unref(im);
    webkit_settings_set_enable_developer_extras(webkit_web_view_get_settings(p->web), FALSE);
    g_signal_connect(p->web, "notify::uri", G_CALLBACK(changed), p);
    g_signal_connect(p->web, "notify::title", G_CALLBACK(changed), p);
    g_signal_connect(p->web, "notify::is-loading", G_CALLBACK(changed), p);
    g_signal_connect(p->web, "load-changed", G_CALLBACK(loaded), p);
    g_signal_connect(p->web, "load-failed", G_CALLBACK(failed), p);
    g_signal_connect(p->web, "web-process-terminated", G_CALLBACK(terminated), p);
    g_signal_connect(p->web, "decide-policy", G_CALLBACK(policy), p);
    g_signal_connect(p->web, "permission-request", G_CALLBACK(permission), p);
    g_signal_connect(p->web, "context-menu", G_CALLBACK(context_menu), p);
    g_signal_connect(p->web, "show-option-menu", G_CALLBACK(option_menu), p);
    g_hash_table_insert(pages, GUINT_TO_POINTER(id), p);
    return p;
}
static double number(JsonObject *o, const char *key) {
    return json_object_has_member(o, key) ? json_object_get_double_member(o, key) : 0;
}
static const char *string(JsonObject *o, const char *key) {
    return json_object_has_member(o, key) ? json_object_get_string_member(o, key) : "";
}
// GPUI sends GDK's modifier bits; the pointer buttons already agree.
static WPEModifiers modifiers(guint mods) {
    return (mods & (1 << 0) ? WPE_MODIFIER_KEYBOARD_SHIFT : 0) |
           (mods & (1 << 2) ? WPE_MODIFIER_KEYBOARD_CONTROL : 0) |
           (mods & (1 << 3) ? WPE_MODIFIER_KEYBOARD_ALT : 0) |
           (mods & (1 << 26) ? WPE_MODIFIER_KEYBOARD_META : 0) | (mods & (0x1f << 8));
}
// GPUI sends wheel deltas in GDK smooth-scroll units. Measured against the
// page, this factor moves it as far per unit as WebKitGTK did.
#define SCROLL_UNIT 48.0
static void input_event(Page *p, JsonObject *o, const char *command) {
    guint32 time = (guint32)(g_get_monotonic_time() / 1000);
    WPEModifiers mods = modifiers(number(o, "mods"));
    double x = number(o, "x") * p->scale, y = number(o, "y") * p->scale;
    WPEEvent *event = NULL;
    if (!strcmp(command, "move")) {
        event = wpe_event_pointer_move_new(WPE_EVENT_POINTER_MOVE, p->view, WPE_INPUT_SOURCE_MOUSE,
                                           time, mods, x, y, 0, 0);
    } else if (!strcmp(command, "down") || !strcmp(command, "up")) {
        gboolean down = *command == 'd';
        guint button = number(o, "button");
        if (down) {
            p->press_x = x;
            p->press_y = y;
            wpe_view_focus_in(p->view);
        }
        event = wpe_event_pointer_button_new(
            down ? WPE_EVENT_POINTER_DOWN : WPE_EVENT_POINTER_UP, p->view, WPE_INPUT_SOURCE_MOUSE,
            time, mods, button, x, y,
            down ? wpe_view_compute_press_count(p->view, x, y, button, time) : 0);
    } else if (!strcmp(command, "scroll")) {
        event = wpe_event_scroll_new(p->view, WPE_INPUT_SOURCE_MOUSE, time, mods,
                                     -number(o, "dx") * SCROLL_UNIT, -number(o, "dy") * SCROLL_UNIT,
                                     TRUE, FALSE, x, y);
    } else if (!strcmp(command, "key_down") || !strcmp(command, "key_up")) {
        const char *name = string(o, "key");
        guint keyval = xkb_keysym_from_name(name, XKB_KEYSYM_NO_FLAGS);
        if (keyval == XKB_KEY_NoSymbol) {
            gunichar u = g_utf8_get_char_validated(name, -1);
            if (u != (gunichar)-1 && u != (gunichar)-2)
                keyval = wpe_unicode_to_keyval(u);
        }
        guint keycode = 0;
        WPEKeymapEntry *entries = NULL;
        guint count = 0;
        if (wpe_keymap_get_entries_for_keyval(keymap, keyval, &entries, &count) && count) {
            keycode = entries[0].keycode;
            g_free(entries);
        }
        event = wpe_event_keyboard_new(
            !strcmp(command, "key_down") ? WPE_EVENT_KEYBOARD_KEY_DOWN : WPE_EVENT_KEYBOARD_KEY_UP,
            p->view, WPE_INPUT_SOURCE_KEYBOARD, time, mods, keycode, keyval);
    }
    if (event) {
        wpe_view_event(p->view, event);
        wpe_event_unref(event);
    }
}
static void copied(GObject *web, GAsyncResult *result, gpointer data) {
    GError *error = NULL;
    JSCValue *v = webkit_web_view_evaluate_javascript_finish(WEBKIT_WEB_VIEW(web), result, &error);
    if (v) {
        gchar *text = jsc_value_to_string(v);
        send_packet('C', GPOINTER_TO_UINT(data), text, strlen(text));
        g_free(text);
        g_object_unref(v);
    }
    g_clear_error(&error);
}
static void evaluated(GObject *web, GAsyncResult *result, gpointer data) {
    guint id = GPOINTER_TO_UINT(data);
    GError *error = NULL;
    JSCValue *v = webkit_web_view_evaluate_javascript_finish(WEBKIT_WEB_VIEW(web), result, &error);
    gchar *s = v ? jsc_value_to_json(v, 0) : g_strdup("null");
    send_packet('J', id, s ?: "null", strlen(s ?: "null"));
    g_free(s);
    g_clear_object(&v);
    g_clear_error(&error);
}
static void command(JsonObject *o) {
    guint id = number(o, "id");
    const char *cmd = string(o, "cmd");
    Page *p = g_hash_table_lookup(pages, GUINT_TO_POINTER(id));
    if (!strcmp(cmd, "create")) {
        if (!p)
            new_page(id);
        return;
    }
    if (!p)
        return;
    if (!strcmp(cmd, "close")) {
        g_hash_table_remove(pages, GUINT_TO_POINTER(id));
        return;
    }
    if (!strcmp(cmd, "load")) {
        const char *url = string(o, "url");
        if (allowed(url)) {
            p->local = FALSE;
            webkit_web_view_load_uri(p->web, url);
        }
    } else if (!strcmp(cmd, "html")) {
        // One command is bounded by MAX_COMMAND; a document arrives in chunks.
        const char *data = string(o, "data");
        if (!p->html)
            p->html = g_string_new(NULL);
        if (p->html->len + strlen(data) <= MAX_HTML)
            g_string_append(p->html, data);
    } else if (!strcmp(cmd, "load-html")) {
        p->local = TRUE;
        webkit_web_view_load_html(p->web, p->html ? p->html->str : "", NULL);
        if (p->html)
            g_string_truncate(p->html, 0);
    } else if (!strcmp(cmd, "dismiss-menu")) {
        if (p->options) {
            webkit_option_menu_close(p->options);
            g_clear_object(&p->options);
        }
    } else if (g_str_has_prefix(cmd, "option:")) {
        if (p->options) {
            guint index = g_ascii_strtoull(cmd + 7, NULL, 10);
            if (index < webkit_option_menu_get_n_items(p->options))
                webkit_option_menu_activate_item(p->options, index);
            webkit_option_menu_close(p->options);
            g_clear_object(&p->options);
        }
    } else if (!strcmp(cmd, "open-link")) {
        if (p->context_link && allowed(p->context_link))
            send_packet('N', p->id, p->context_link, strlen(p->context_link));
    } else if (!strcmp(cmd, "copy-link")) {
        if (p->context_link)
            send_packet('C', p->id, p->context_link, strlen(p->context_link));
    } else if (!strcmp(cmd, "select-all"))
        webkit_web_view_execute_editing_command(p->web, "SelectAll");
    else if (!strcmp(cmd, "reload"))
        webkit_web_view_reload(p->web);
    else if (!strcmp(cmd, "commit") || !strcmp(cmd, "preedit") || !strcmp(cmd, "unmark")) {
        BrowserIM *im = (BrowserIM *)webkit_web_view_get_input_method_context(p->web);
        if (!strcmp(cmd, "preedit")) {
            if (!*im->preedit)
                g_signal_emit_by_name(im, "preedit-started");
            g_free(im->preedit);
            im->preedit = g_strdup(string(o, "text"));
            im->cursor = g_utf8_strlen(im->preedit, -1);
            g_signal_emit_by_name(im, "preedit-changed");
        } else {
            if (!strcmp(cmd, "commit"))
                g_signal_emit_by_name(im, "committed", string(o, "text"));
            g_free(im->preedit);
            im->preedit = g_strdup("");
            im->cursor = 0;
            g_signal_emit_by_name(im, "preedit-changed");
            g_signal_emit_by_name(im, "preedit-finished");
        }
    } else if (!strcmp(cmd, "text"))
        webkit_web_view_execute_editing_command_with_argument(p->web, "InsertText",
                                                              string(o, "text"));
    else if (!strcmp(cmd, "copy") || !strcmp(cmd, "cut")) {
        const char *script =
            !strcmp(cmd, "cut")
                ? "(()=>{let e=document.activeElement;let s=e&&e.type==='password'?'':e&&typeof "
                  "e.selectionStart==='number'?e.value.slice(e.selectionStart,e.selectionEnd):"
                  "String(getSelection());if(s)document.execCommand('delete');return s})()"
                : "(()=>{let e=document.activeElement;return e&&e.type==='password'?'':e&&typeof "
                  "e.selectionStart==='number'?e.value.slice(e.selectionStart,e.selectionEnd):"
                  "String(getSelection())})()";
        webkit_web_view_evaluate_javascript(p->web, script, -1, NULL, NULL, NULL, copied,
                                            GUINT_TO_POINTER(id));
    } else if (!strcmp(cmd, "back"))
        webkit_web_view_go_back(p->web);
    else if (!strcmp(cmd, "forward"))
        webkit_web_view_go_forward(p->web);
    else if (!strcmp(cmd, "resize")) {
        guint w = CLAMP(number(o, "width"), 1, MAX_DIMENSION),
              h = CLAMP(number(o, "height"), 1, MAX_DIMENSION);
        double scale = CLAMP(number(o, "scale"), 0.5, 4);
        if (w != p->width || h != p->height || scale != p->scale) {
            p->width = w;
            p->height = h;
            p->scale = scale;
            webkit_web_view_set_zoom_level(p->web, scale);
            wpe_toplevel_resized(p->toplevel, w, h);
            wpe_view_resized(p->view, w, h);
        }
    } else if (!strcmp(cmd, "visible")) {
        // A hidden page stops being composited instead of painting unseen frames.
        p->visible = number(o, "value") != 0;
        wpe_view_set_visible(p->view, p->visible);
        if (p->visible)
            wpe_view_map(p->view);
        else
            wpe_view_unmap(p->view);
    } else if (!strcmp(cmd, "eval"))
        webkit_web_view_evaluate_javascript(p->web, string(o, "script"), -1, NULL, NULL, NULL,
                                            evaluated, GUINT_TO_POINTER(id));
    else
        input_event(p, o, cmd);
}
static gboolean read_commands(gint fd, GIOCondition condition, gpointer unused) {
    guint8 bytes[65536];
    ssize_t n = read(fd, bytes, sizeof bytes);
    if (n <= 0) {
        g_main_loop_quit(main_loop);
        return G_SOURCE_REMOVE;
    }
    g_byte_array_append(input, bytes, n);
    while (input->len >= 4) {
        guint32 length;
        memcpy(&length, input->data, 4);
        length = GUINT32_FROM_LE(length);
        if (length > MAX_COMMAND) {
            g_main_loop_quit(main_loop);
            return G_SOURCE_REMOVE;
        }
        if (input->len < 4 + length)
            break;
        JsonParser *parser = json_parser_new();
        if (json_parser_load_from_data(parser, (char *)input->data + 4, length, NULL)) {
            JsonNode *root = json_parser_get_root(parser);
            if (JSON_NODE_HOLDS_OBJECT(root))
                command(json_node_get_object(root));
        }
        g_object_unref(parser);
        g_byte_array_remove_range(input, 0, length + 4);
    }
    return G_SOURCE_CONTINUE;
}
int main(int argc, char **argv) {
    signal(SIGPIPE, SIG_IGN);
    // A frame is megabytes; the default 64 KiB pipe costs a wakeup per chunk.
    fcntl(STDOUT_FILENO, F_SETPIPE_SZ, 1024 * 1024);
    // The display's rate is not visible from here; the host may state it.
    const char *hz = g_getenv("ZERON_BROWSER_REFRESH_HZ");
    double rate = hz ? g_ascii_strtod(hz, NULL) : 0;
    refresh_mhz = (rate >= 24 && rate <= 480 ? rate : DEFAULT_REFRESH_HZ) * 1000;
    screen = g_object_new(browser_screen_get_type(), "id", 1, NULL);
    wpe_screen_set_size(screen, MAX_DIMENSION, MAX_DIMENSION);
    wpe_screen_set_scale(screen, 1);
    wpe_screen_set_refresh_rate(screen, refresh_mhz);
    keymap = wpe_keymap_xkb_new();
    display = g_object_new(browser_display_get_type(), NULL);
    GError *error = NULL;
    if (!wpe_display_connect(display, &error)) {
        g_printerr("Could not start the WPE WebKit display: %s\n", error->message);
        return 1;
    }
    pages = g_hash_table_new_full(g_direct_hash, g_direct_equal, NULL, free_page);
    input = g_byte_array_new();
    session = webkit_network_session_new_ephemeral();
    g_signal_connect(session, "download-started", G_CALLBACK(download), NULL);
    g_unix_fd_add(STDIN_FILENO, G_IO_IN | G_IO_HUP | G_IO_ERR, read_commands, NULL);
    GMainLoop *loop = g_main_loop_new(NULL, FALSE);
    main_loop = loop;
    g_main_loop_run(loop);
    g_hash_table_destroy(pages);
    g_byte_array_unref(input);
    g_object_unref(session);
    return 0;
}
