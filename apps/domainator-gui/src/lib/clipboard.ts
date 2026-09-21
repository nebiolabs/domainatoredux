/// Copy `text` to the clipboard, returning whether it worked.
///
/// `navigator.clipboard` is not dependable in these webviews — it is absent or
/// rejects under some custom-protocol origins on WebKitGTK — so fall back to a
/// hidden textarea plus the legacy command, which is deprecated but still
/// implemented in WebView2, WebKitGTK and WKWebView.
///
/// Callers must handle `false`: the whole point of "Copy Path" is that the path
/// is otherwise unreachable, so failing silently would leave the user with
/// nothing.
export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    /* fall through to the selection-based path */
  }
  try {
    const ta = document.createElement("textarea");
    ta.value = text;
    // Off-screen but still focusable: execCommand("copy") needs a real selection
    // in the document, and `display: none` would not give us one.
    ta.setAttribute("readonly", "");
    ta.style.position = "fixed";
    ta.style.top = "-1000px";
    ta.style.opacity = "0";
    document.body.appendChild(ta);
    ta.select();
    const ok = document.execCommand("copy");
    document.body.removeChild(ta);
    return ok;
  } catch {
    return false;
  }
}
