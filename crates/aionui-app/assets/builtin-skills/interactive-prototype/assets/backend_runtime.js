(function () {
  function ensureToastStack() {
    let stack = document.querySelector(".ip-toast-stack");
    if (!stack) {
      stack = document.createElement("div");
      stack.className = "ip-toast-stack";
      document.body.appendChild(stack);
    }
    return stack;
  }

  function showToast(message) {
    const stack = ensureToastStack();
    const toast = document.createElement("div");
    toast.className = "ip-toast";
    toast.textContent = message;
    stack.appendChild(toast);
    window.setTimeout(() => toast.remove(), 2200);
  }

  function toggleLayer(targetId, hidden) {
    if (!targetId) return;
    const layer = document.getElementById(targetId);
    if (!layer) return;
    layer.classList.toggle("is-hidden", hidden);
  }

  function setActiveTab(groupName, tabTarget) {
    document.querySelectorAll(`[data-tab-group="${groupName}"]`).forEach((button) => {
      button.classList.toggle("is-active", button.getAttribute("data-tab-target") === tabTarget);
    });

    document.querySelectorAll(`[data-tab-panel-group="${groupName}"]`).forEach((panel) => {
      panel.classList.toggle("is-hidden", panel.getAttribute("data-tab-panel") !== tabTarget);
    });
  }

  document.addEventListener("click", (event) => {
    const navTarget = event.target.closest("[data-nav]");
    if (navTarget) {
      event.preventDefault();
      const target = navTarget.getAttribute("data-nav");
      if (target) window.location.href = target;
      return;
    }

    const openTarget = event.target.closest("[data-open]");
    if (openTarget) {
      event.preventDefault();
      toggleLayer(openTarget.getAttribute("data-open"), false);
      return;
    }

    const closeTarget = event.target.closest("[data-close]");
    if (closeTarget) {
      event.preventDefault();
      toggleLayer(closeTarget.getAttribute("data-close"), true);
      return;
    }

    const toastTarget = event.target.closest("[data-toast]");
    if (toastTarget) {
      event.preventDefault();
      showToast(toastTarget.getAttribute("data-toast") || "操作成功");
      return;
    }

    const tabTarget = event.target.closest("[data-tab-group][data-tab-target]");
    if (tabTarget) {
      event.preventDefault();
      setActiveTab(tabTarget.getAttribute("data-tab-group"), tabTarget.getAttribute("data-tab-target"));
      return;
    }

    const layerTarget = event.target.closest(".ip-layer");
    if (layerTarget && event.target === layerTarget) {
      layerTarget.classList.add("is-hidden");
    }
  });

  document.addEventListener("keydown", (event) => {
    if (event.key !== "Escape") return;
    document.querySelectorAll(".ip-layer").forEach((layer) => {
      layer.classList.add("is-hidden");
    });
  });

  window.InteractivePrototypeRuntime = {
    showToast,
    open(targetId) {
      toggleLayer(targetId, false);
    },
    close(targetId) {
      toggleLayer(targetId, true);
    },
    switchTab(groupName, tabTarget) {
      setActiveTab(groupName, tabTarget);
    },
  };
})();
