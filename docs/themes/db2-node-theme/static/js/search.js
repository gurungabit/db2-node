(function () {
  function normalize(value) {
    return (value || "").toLowerCase().replace(/\s+/g, " ").trim();
  }

  function snippetFor(item, query) {
    var text = item.summary || item.content || "";
    if (!text) {
      return "";
    }

    var normalized = normalize(text);
    var idx = normalized.indexOf(query);
    if (idx === -1) {
      return text.slice(0, 140).trim();
    }

    var start = Math.max(0, idx - 50);
    var end = Math.min(text.length, idx + query.length + 90);
    return (start > 0 ? "..." : "") + text.slice(start, end).trim() + (end < text.length ? "..." : "");
  }

  function scoreItem(item, query) {
    var title = normalize(item.title);
    var section = normalize(item.section);
    var summary = normalize(item.summary);
    var content = normalize(item.content);
    var score = 0;

    if (title.indexOf(query) !== -1) {
      score += 8;
      if (title.indexOf(query) === 0) {
        score += 3;
      }
    }
    if (section.indexOf(query) !== -1) {
      score += 3;
    }
    if (summary.indexOf(query) !== -1) {
      score += 2;
    }
    if (content.indexOf(query) !== -1) {
      score += 1;
    }

    return score;
  }

  document.addEventListener("DOMContentLoaded", function () {
    var shell = document.querySelector("[data-search-shell]");
    if (!shell) {
      return;
    }

    var input = shell.querySelector(".search-input");
    var results = shell.querySelector("[data-search-results]");
    var indexUrl = shell.getAttribute("data-search-index");
    var docs = null;
    var loadPromise = null;
    var debounceTimer = null;

    function closeResults() {
      results.classList.remove("is-open");
      results.innerHTML = "";
    }

    function showMessage(message) {
      results.innerHTML = "";
      var node = document.createElement("div");
      node.className = "search-empty";
      node.textContent = message;
      results.appendChild(node);
      results.classList.add("is-open");
    }

    function ensureIndex() {
      if (docs) {
        return Promise.resolve(docs);
      }
      if (!loadPromise) {
        loadPromise = fetch(indexUrl, { headers: { Accept: "application/json" } })
          .then(function (response) {
            if (!response.ok) {
              throw new Error("Search index request failed");
            }
            return response.json();
          })
          .then(function (payload) {
            docs = Array.isArray(payload) ? payload : [];
            return docs;
          });
      }
      return loadPromise;
    }

    function render(query) {
      if (!query) {
        closeResults();
        return;
      }

      ensureIndex()
        .then(function (items) {
          var matches = items
            .map(function (item) {
              return { item: item, score: scoreItem(item, query) };
            })
            .filter(function (entry) {
              return entry.score > 0;
            })
            .sort(function (a, b) {
              return b.score - a.score;
            })
            .slice(0, 8);

          if (!matches.length) {
            showMessage("No matching docs yet.");
            return;
          }

          var list = document.createElement("ul");
          list.className = "search-results-list";

          matches.forEach(function (entry) {
            var item = entry.item;
            var li = document.createElement("li");
            li.className = "search-results-item";

            var link = document.createElement("a");
            link.className = "search-results-link";
            link.href = item.relPermalink;

            var title = document.createElement("span");
            title.className = "search-results-title";
            title.textContent = item.title;
            link.appendChild(title);

            var meta = document.createElement("span");
            meta.className = "search-results-meta";
            meta.textContent = item.section || item.kind || "page";
            link.appendChild(meta);

            var snippet = snippetFor(item, query);
            if (snippet) {
              var snippetNode = document.createElement("span");
              snippetNode.className = "search-results-snippet";
              snippetNode.textContent = snippet;
              link.appendChild(snippetNode);
            }

            li.appendChild(link);
            list.appendChild(li);
          });

          results.innerHTML = "";
          results.appendChild(list);
          results.classList.add("is-open");
        })
        .catch(function () {
          showMessage("Search index unavailable.");
        });
    }

    input.addEventListener("focus", function () {
      if (input.value.trim()) {
        render(normalize(input.value));
      }
    });

    input.addEventListener("input", function () {
      var query = normalize(input.value);
      window.clearTimeout(debounceTimer);
      debounceTimer = window.setTimeout(function () {
        render(query);
      }, 80);
    });

    input.addEventListener("keydown", function (event) {
      if (event.key === "Escape") {
        input.blur();
        closeResults();
      }
    });

    document.addEventListener("click", function (event) {
      if (!shell.contains(event.target)) {
        closeResults();
      }
    });
  });
})();
