// Inline SVG icons; no icon library, keeps the bundle self-contained.

export const plusOutlined = (
  <svg
    viewBox="64 64 896 896"
    focusable="false"
    fill="currentColor"
    height="14px"
    aria-hidden="true"
  >
    <path d="M328 544h152v152c0 4.4 3.6 8 8 8h48c4.4 0 8-3.6 8-8V544h152c4.4 0 8-3.6 8-8v-48c0-4.4-3.6-8-8-8H544V328c0-4.4-3.6-8-8-8h-48c-4.4 0-8 3.6-8 8v152H328c-4.4 0-8 3.6-8 8v48c0 4.4 3.6 8 8 8z"></path>
    <path d="M880 112H144c-17.7 0-32 14.3-32 32v736c0 17.7 14.3 32 32 32h736c17.7 0 32-14.3 32-32V144c0-17.7-14.3-32-32-32zm-40 728H184V184h656v656z"></path>
  </svg>
);

export const minusOutlined = (
  <svg
    viewBox="64 64 896 896"
    focusable="false"
    fill="currentColor"
    height="14px"
    aria-hidden="true"
  >
    <path d="M328 544h368c4.4 0 8-3.6 8-8v-48c0-4.4-3.6-8-8-8H328c-4.4 0-8 3.6-8 8v48c0 4.4 3.6 8 8 8z"></path>
    <path d="M880 112H144c-17.7 0-32 14.3-32 32v736c0 17.7 14.3 32 32 32h736c17.7 0 32-14.3 32-32V144c0-17.7-14.3-32-32-32zm-40 728H184V184h656v656z"></path>
  </svg>
);

export const getLogoIcon = (isDarkMode: boolean) => {
  const color = isDarkMode ? "#2dd4bf" : "#0d9488";
  return (
    <svg
      height="28"
      viewBox="0 0 64 64"
      xmlns="http://www.w3.org/2000/svg"
      style={{
        fill: color,
        display: "block",
        flexShrink: 0,
      }}
    >
      <path d="m27.04 24.126c.419-.293.977-.288 1.39.013l7.807 5.681c4.489 3.265 10.827 2.623 14.43-1.465 2.143-2.431 3.324-5.553 3.324-8.791 0-7.889-6.359-14.308-14.174-14.308h-25.397c-7.4 0-13.42 6.077-13.42 13.546v.762c0 4.31 2.104 8.369 5.627 10.859 1.685 1.191 3.671 1.785 5.669 1.785 2.028-.001 4.069-.613 5.82-1.838zm-18.578 3.7c-2.682-1.895-4.282-4.983-4.282-8.262v-.762c0-5.716 4.594-10.366 10.241-10.366h25.397c6.063 0 10.995 4.992 10.995 11.128 0 2.463-.898 4.839-2.53 6.688-2.531 2.868-6.999 3.307-10.174.997l-8.046-5.855c-1.368-.995-3.217-1.012-4.603-.043l-9.166 6.414c-2.379 1.665-5.528 1.69-7.832.061z" />
      <path d="m29.679 19.546c2.333 1.677 6.026 4.332 8.501 6.11 2.42 1.739 5.768 1.776 8.047-.144 1.85-1.558 3.029-3.887 3.029-6.478 0-4.955-4.054-9.009-9.009-9.009h-25.36c-4.745 0-8.627 3.882-8.627 8.627v.382c0 2.358.978 4.5 2.548 6.041 2.323 2.279 6.024 2.379 8.629.428l7.896-5.912c1.285-.962 3.042-.982 4.346-.045z" />
      <path d="m62.973 1.017h-7.419c.007.177.027.351.027.53v40.274c0 7.305-5.943 13.248-13.248 13.248-6.765 0-12.337-5.103-13.126-11.658h6.238v-8.479h-19.077v8.479h5.406c.819 10.65 9.703 19.077 20.56 19.077 11.395-.001 20.666-9.272 20.666-20.667v-40.274c0-.179-.022-.352-.027-.53zm-2.093 9.539h-3.179v-8.479h3.179z" />
    </svg>
  );
};

export const getGithubIcon = (isDarkMode: boolean) => {
  if (window.location.host.indexOf("diving") === -1) {
    return;
  }
  const color = isDarkMode ? "#e8eef2" : "#0f1c24";
  return (
    <a
      className="githubCorner"
      href="https://github.com/vicanso/diving-rs"
      style={{
        position: "fixed",
        padding: "14px 18px",
        right: 0,
        top: 0,
        zIndex: 120,
        lineHeight: 0,
      }}
    >
      <svg
        height="28"
        viewBox="0 0 16 16"
        width="28"
        aria-hidden="true"
        style={{
          fill: color,
        }}
      >
        <path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0 0 16 8c0-4.42-3.58-8-8-8z" />
      </svg>
    </a>
  );
};

export const getDownloadIcon = () => {
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      style={{ verticalAlign: "middle", color: "var(--accent)" }}
    >
      <path
        d="M12 16L12 8"
        stroke="currentColor"
        strokeWidth="2.5"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <path
        d="M9 13L11.913 15.913V15.913C11.961 15.961 12.039 15.961 12.087 15.913V15.913L15 13"
        stroke="currentColor"
        strokeWidth="2.5"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <path
        d="M3 15L3 16L3 19C3 20.1046 3.89543 21 5 21L19 21C20.1046 21 21 20.1046 21 19L21 16L21 15"
        stroke="currentColor"
        strokeWidth="2.5"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
};
