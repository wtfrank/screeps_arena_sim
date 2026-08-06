In order for the sim to load a bot, there are some entry points that need to be defined. These replace the javascript wasm entry points.

diff --git a/src/lib.rs b/src/lib.rs
index 4fd86e2..8458f02 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -83,3 +83,28 @@ impl Bot {
         }
     }
 }
+
+#[cfg(not(target_arch = "wasm32"))]
+#[unsafe(no_mangle)]
+pub unsafe extern "C" fn bot_initialize() -> *mut Bot {
+    Box::into_raw(Box::new(Bot::initialize()))
+}
+
+#[cfg(not(target_arch = "wasm32"))]
+#[unsafe(no_mangle)]
+pub unsafe extern "C" fn bot_tick(bot: *mut Bot) {
+    unsafe {
+        let bot = &mut *bot;
+        bot.profile_tick();
+    }
+}
+
+#[cfg(not(target_arch = "wasm32"))]
+#[unsafe(no_mangle)]
+pub unsafe extern "C" fn bot_free(bot: *mut Bot) {
+    unsafe {
+        if !bot.is_null() {
+            let _ = Box::from_raw(bot);
+        }
+    }
+}
