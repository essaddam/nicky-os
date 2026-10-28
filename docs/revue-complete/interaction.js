(() => {
  'use strict';
  const select = (value, parent = document) => parent.querySelector(value);
  const selectAll = (value, parent = document) => [...parent.querySelectorAll(value)];
  const normalize = value => value.normalize('NFD').replace(/[\u0300-\u036f]/g, '').toLowerCase();
  const ideas = selectAll('.idea');
  const ideaText = new Map(ideas.map(idea => [idea, normalize(idea.textContent)]));
  const query = select('#search');
  const state = select('#status-filter');
  const theme = select('#theme-filter');
  const topics = selectAll('.topic');
  const originalHosts = new Map(topics.map(topic => [topic, topic.parentElement]));
  const tabs = selectAll('[role="tab"][data-tab]');
  const panels = selectAll('.theme-panel');
  let activeTheme = tabs[0].dataset.tab;
  function activateTheme(id) {
    if (!tabs.some(tab => tab.dataset.tab === id)) return;
    activeTheme = id;
    if (id === 'tout') {
      topics.sort((a, b) => Number(a.dataset.order) - Number(b.dataset.order))
        .forEach(topic => select('#all-topic-content').append(topic));
    } else {
      topics.forEach(topic => originalHosts.get(topic).append(topic));
    }
    panels.forEach(panel => { panel.hidden = panel.id !== `theme-${id}`; });
    tabs.forEach(tab => {
      const selected = tab.dataset.tab === id;
      tab.setAttribute('aria-selected', String(selected));
      tab.tabIndex = selected ? 0 : -1;
    });
    const selectedTab = tabs.find(tab => tab.dataset.tab === id);
    const tabbar = select('.theme-tabs');
    if (tabbar.scrollWidth > tabbar.clientWidth) {
      tabbar.scrollLeft = selectedTab.offsetLeft - (tabbar.clientWidth - selectedTab.offsetWidth) / 2;
    }
  }
  function clearFilters() { query.value = ''; state.value = ''; theme.value = ''; }
  function filterIdeas() {
    const terms = normalize(query.value).trim().split(/\s+/).filter(Boolean);
    let count = 0;
    ideas.forEach(idea => {
      const show = terms.every(term => ideaText.get(idea).includes(term))
        && (!state.value || idea.dataset.status === state.value)
        && (!theme.value || idea.closest('.topic').id === theme.value)
        && (activeTheme === 'tout' || idea.closest('.topic').dataset.theme === activeTheme);
      idea.hidden = !show;
      if (show) count += 1;
    });
    topics.forEach(topic => { topic.hidden = !selectAll('.idea', topic).some(idea => !idea.hidden); });
    select('#filter-count').textContent = `${count} idée${count > 1 ? 's' : ''} affichée${count > 1 ? 's' : ''} sur ${ideas.length}`;
    select('#ideas-empty').hidden = count !== 0;
  }
  [query, state, theme].forEach(input => input.addEventListener('input', () => { activateTheme('tout'); filterIdeas(); }));
  select('#reset-filters').addEventListener('click', () => { clearFilters(); activateTheme('tout'); filterIdeas(); });
  function chooseTab(tab, keyboard = false) {
    clearFilters();
    activateTheme(tab.dataset.tab);
    filterIdeas();
    const panel = select(`#theme-${tab.dataset.tab}`);
    if (location.hash !== `#${panel.id}`) history.pushState(null, '', `#${panel.id}`);
    if (keyboard) tab.focus({ preventScroll: true });
    requestAnimationFrame(() => panel.scrollIntoView({ block: 'start' }));
  }
  tabs.forEach((tab, index) => {
    tab.addEventListener('click', () => chooseTab(tab));
    tab.addEventListener('keydown', event => {
      let next;
      if (event.key === 'ArrowRight') next = (index + 1) % tabs.length;
      if (event.key === 'ArrowLeft') next = (index - 1 + tabs.length) % tabs.length;
      if (event.key === 'Home') next = 0;
      if (event.key === 'End') next = tabs.length - 1;
      if (next !== undefined) { event.preventDefault(); chooseTab(tabs[next], true); }
    });
  });
  selectAll('[data-depth]').forEach(button => button.addEventListener('click', () => {
    ideas.filter(idea => !idea.hidden).forEach(idea => { idea.open = button.dataset.depth === 'detail'; });
  }));
  function revealHash() {
    const id = decodeURIComponent(location.hash.slice(1));
    const target = id && document.getElementById(id);
    if (!target) return;
    const topic = target.closest('.topic');
    const panel = target.matches('.theme-panel') ? target : null;
    if (topic || panel) {
      clearFilters();
      activateTheme(topic ? topic.dataset.theme : panel.id.slice('theme-'.length));
      filterIdeas();
      requestAnimationFrame(() => target.scrollIntoView({ block: 'start' }));
    }
    const idea = target.closest('.idea');
    if (idea) {
      idea.open = true;
      requestAnimationFrame(() => idea.scrollIntoView({ block: 'start' }));
    }
  }
  window.addEventListener('hashchange', revealHash);
  const sidebar = select('.sidebar');
  const menu = select('#menu-toggle');
  menu.addEventListener('click', () => { const opened = sidebar.classList.toggle('open'); menu.setAttribute('aria-expanded', String(opened)); });
  selectAll('.sidebar a').forEach(link => link.addEventListener('click', () => { sidebar.classList.remove('open'); menu.setAttribute('aria-expanded', 'false'); }));
  document.addEventListener('keydown', event => { if (event.key === 'Escape') { sidebar.classList.remove('open'); menu.setAttribute('aria-expanded', 'false'); } });
  const sourceData = JSON.parse(select('#source-data').textContent);
  const sourceDialog = select('#source-dialog');
  function openSource(path, line) {
    const source = sourceData[path];
    if (!source) return;
    select('#source-title').textContent = `${path} · ligne ${line}`;
    const container = select('#source-lines');
    container.replaceChildren();
    source.text.split('\n').forEach((text, index) => {
      const number = index + (source.start || 1);
      const row = document.createElement('span');
      row.className = 'source-line' + (number === line ? ' selected-line' : '');
      const no = document.createElement('span');
      no.className = 'line-no';
      no.textContent = String(number);
      row.append(no, document.createTextNode(text || ' '));
      container.append(row);
    });
    select('#source-original').href = '../../' + path;
    sourceDialog.showModal();
    requestAnimationFrame(() => { const row = select('.selected-line', container); if (row) row.scrollIntoView({ block: 'center' }); });
  }
  document.addEventListener('click', event => {
    const source = event.target.closest('[data-source]');
    if (source) openSource(source.dataset.source, Number(source.dataset.line || 1));
    const figure = event.target.closest('[data-figure]');
    if (figure) {
      const img = select('img', figure);
      select('#zoom-image').src = img.src;
      select('#zoom-image').alt = img.alt;
      select('#zoom-title').textContent = img.alt;
      select('#figure-dialog').showModal();
    }
    const close = event.target.closest('[data-close]');
    if (close) close.closest('dialog').close();
  });
  selectAll('dialog').forEach(dialog => dialog.addEventListener('click', event => { if (event.target === dialog) dialog.close(); }));
  const gallerySearch = select('#gallery-search');
  const galleryState = select('#gallery-state');
  const figures = selectAll('#gallery .figure');
  const figureText = new Map(figures.map(figure => [figure, normalize(figure.textContent)]));
  function filterGallery() {
    const terms = normalize(gallerySearch.value).trim().split(/\s+/).filter(Boolean);
    let count = 0;
    figures.forEach(figure => { figure.hidden = !terms.every(term => figureText.get(figure).includes(term)) || (galleryState.value && figure.dataset.history !== galleryState.value); if (!figure.hidden) count += 1; });
    select('#gallery-count').textContent = `${count} figure${count > 1 ? 's' : ''} affichée${count > 1 ? 's' : ''}`;
  }
  [gallerySearch, galleryState].forEach(input => input.addEventListener('input', filterGallery));
  const sources = selectAll('.source-index');
  select('#source-search').addEventListener('input', event => {
    const terms = normalize(event.target.value).trim().split(/\s+/).filter(Boolean);
    sources.forEach(source => { source.hidden = !terms.every(term => normalize(source.textContent).includes(term)); });
  });
  select('#print').addEventListener('click', () => window.print());
  let printState;
  let printTheme;
  window.addEventListener('beforeprint', () => {
    printTheme = activeTheme;
    printState = selectAll('details').map(detail => [detail, detail.open, detail.hidden]);
    topics.forEach(topic => { originalHosts.get(topic).append(topic); topic.hidden = false; });
    panels.forEach(panel => { panel.hidden = panel.id === 'theme-tout'; });
    printState.forEach(([detail]) => { detail.open = true; detail.hidden = false; });
  });
  window.addEventListener('afterprint', () => {
    if (printState) printState.forEach(([detail, open, hidden]) => { detail.open = open; detail.hidden = hidden; });
    activateTheme(printTheme || activeTheme);
    filterIdeas();
  });
  const observer = new IntersectionObserver(entries => {
    entries.filter(entry => entry.isIntersecting).forEach(entry => { selectAll('.sidebar nav a').forEach(link => link.classList.toggle('active', link.hash === '#' + entry.target.id)); });
  }, { rootMargin: '-5% 0px -75% 0px' });
  selectAll('section[id]').forEach(section => observer.observe(section));
  activateTheme(activeTheme); filterIdeas(); filterGallery(); revealHash();
})();
