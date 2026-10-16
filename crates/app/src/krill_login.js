(() => {
  if (window.top !== window || location.origin !== "https://www.krill-code.com") return null;
  const signedIn = ["/app", "/admin", "/advisor", "/support"].some(
    (path) => location.pathname === path || location.pathname.startsWith(`${path}/`),
  );
  if (!signedIn) return null;
  const token = localStorage.getItem("krill_jwt");
  return token && token.length <= 16384 ? token : null;
})()
