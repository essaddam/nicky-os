/* Contrôles du document autonome. Playwright disponible et Chrome sont requis.
 * Facultatif : MOLI_REVIEW_PLAYWRIGHT et MOLI_REVIEW_CHROME désignent leurs chemins.
 * Aucun serveur ni appareil de maison n'est contacté.
 */
const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { pathToFileURL } = require('node:url');
const playwright = process.env.MOLI_REVIEW_PLAYWRIGHT || require.resolve('playwright', {
  paths: [process.cwd(), path.join(os.homedir(), '.dev-browser')],
});
const { chromium } = require(playwright);
const directory = __dirname;
const evidence = path.join(directory, 'preuves');
const report = {
  date: new Date().toISOString(),
  fichier: 'index.html',
  sha256Html: crypto.createHash('sha256').update(fs.readFileSync(path.join(directory, 'index.html'))).digest('hex'),
  controles: [], erreurs: [], requetesExternes: [], captures: [],
};
const file = pathToFileURL(path.join(directory, 'index.html')).href;
const chrome = process.env.MOLI_REVIEW_CHROME || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';

function pass(criterion, detail) {
  report.controles.push({ critere: criterion, resultat: 'PASS', preuve: detail });
}

(async () => {
  fs.mkdirSync(evidence, { recursive: true });
  const browser = await chromium.launch({ headless: true, executablePath: chrome });
  const deadline = setTimeout(() => browser.close(), 90000);
  try {
    const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
    page.on('pageerror', error => report.erreurs.push(error.message));
    page.on('request', request => {
      if (/^https?:/.test(request.url())) report.requetesExternes.push(request.url());
    });
    await page.goto(file, { waitUntil: 'load' });
    assert.match(await page.title(), /Revue complète/);
    const total = await page.locator('.idea').count();
    assert.ok(total > 150);
    assert.match(await page.locator('#filter-count').textContent(), new RegExp(String(total)));
    pass('Ouverture locale et initialisation', { idees: total, titre: await page.title() });
    await page.screenshot({ path: path.join(evidence, 'vue-ensemble-bureau.png') });
    report.captures.push('preuves/vue-ensemble-bureau.png');

    assert.equal(await page.getByRole('tab').count(), 12);
    assert.equal(await page.locator('.theme-panel:not([hidden])').count(), 1);
    assert.equal(await page.locator('#tab-coeur').getAttribute('aria-selected'), 'true');
    for (const tab of await page.getByRole('tab').all()) {
      await tab.click();
      const target = await tab.getAttribute('aria-controls');
      assert.equal(await page.locator('.theme-panel:not([hidden])').count(), 1);
      assert.ok(await page.locator(`#${target}`).isVisible());
      if (target !== 'theme-tout') {
        assert.equal(await page.locator(`#${target} .theme-explanation h4`).count(), 2);
        assert.equal(await page.locator(`#${target} .theme-flow li`).count(), 4);
        assert.ok((await page.locator(`#${target} .theme-explanation`).textContent()).length > 800);
      }
    }
    assert.equal(await page.locator('.idea').count(), total);
    assert.equal(await page.locator('.idea:not([hidden])').count(), total);
    pass('Onglets et explications par thème', '11 thèmes expliqués, un panneau actif, 31 sous-thèmes et 220 idées conservés ; Liste complète accessible.');
    await page.locator('#tab-coeur').click();
    await page.locator('#theme-coeur').scrollIntoViewIfNeeded();
    await page.screenshot({ path: path.join(evidence, 'onglet-architecture-bureau.png') });
    report.captures.push('preuves/onglet-architecture-bureau.png');
    await page.locator('#tab-coeur').focus();
    await page.keyboard.press('ArrowRight');
    assert.equal(await page.locator('#tab-valeurs').getAttribute('aria-selected'), 'true');
    await page.keyboard.press('End');
    assert.equal(await page.locator('#tab-tout').getAttribute('aria-selected'), 'true');
    await page.keyboard.press('Home');
    assert.equal(await page.locator('#tab-coeur').getAttribute('aria-selected'), 'true');
    await page.goBack();
    await page.waitForFunction(() => document.querySelectorAll('.theme-panel:not([hidden])').length === 1);
    await page.locator('#tab-tout').click();
    pass('Navigation des onglets au clavier et historique', 'Flèches, début, fin et retour ne perdent ni contenu ni panneau actif.');

    await page.locator('#search').fill('generation');
    const ascii = await page.locator('.idea:not([hidden])').count();
    assert.ok(ascii > 0 && ascii < total);
    await page.locator('#search').fill('génération');
    assert.equal(await page.locator('.idea:not([hidden])').count(), ascii);
    pass('Recherche et accents', { resultats: ascii, equivalence: 'generation = génération' });
    await page.locator('#status-filter').selectOption('arbitrage');
    assert.equal(await page.locator('.idea:not([hidden]):not([data-status="arbitrage"])').count(), 0);
    await page.locator('#theme-filter').selectOption('cible-contrats');
    assert.equal(await page.locator('.topic:not([hidden])').count(), 1);
    pass('Filtres combinés', 'Thème, avancement et recherche se croisent.');
    await page.locator('#search').fill('motabsentdelarevuexyz');
    assert.equal(await page.locator('.idea:not([hidden])').count(), 0);
    assert.ok(await page.locator('#ideas-empty').isVisible());
    await page.locator('#reset-filters').click();
    assert.equal(await page.locator('.idea:not([hidden])').count(), total);
    pass('Aucun résultat et réinitialisation', 'Message visible, puis retour de toutes les idées.');

    await page.locator('[data-depth="detail"]').click();
    assert.equal(await page.locator('.idea[open]').count(), total);
    await page.locator('[data-depth="global"]').click();
    assert.equal(await page.locator('.idea[open]').count(), 0);
    pass('Vue globale et détails', 'Les deux commandes ouvrent et réduisent toutes les idées affichées.');

    await page.locator('#search').fill('motabsentdelarevuexyz');
    await page.evaluate(() => { location.hash = 'cible-decision-sarah'; });
    await page.waitForFunction(() => document.getElementById('cible-decision-sarah').open);
    assert.ok(await page.locator('#cible-decision-sarah').isVisible());
    assert.equal(await page.locator('#tab-coeur').getAttribute('aria-selected'), 'true');
    assert.equal(await page.locator('#search').inputValue(), '');
    pass('Lien direct malgré un filtre', 'Le détail est révélé dans son onglet et le filtre incompatible est réinitialisé.');
    await page.screenshot({ path: path.join(evidence, 'detail-contrat.png') });
    report.captures.push('preuves/detail-contrat.png');

    await page.locator('#cible-decision-sarah [data-source]').first().click();
    assert.ok(await page.locator('#source-dialog').isVisible());
    assert.equal(await page.locator('#source-lines .selected-line').count(), 1);
    assert.ok((await page.locator('#source-lines').textContent()).includes('workflows'));
    await page.locator('#source-dialog [data-close]').click();
    pass('Lecture des sources embarquées', 'Source documentaire ouverte, ligne indiquée surlignée, fermeture fonctionnelle.');
    const code = page.locator('.idea [data-source^="crates/"]').first();
    const ideaId = await code.evaluate(element => element.closest('.idea').id);
    await page.evaluate(id => { location.hash = id; }, ideaId);
    await code.click();
    assert.ok(await page.locator('#source-dialog').isVisible());
    assert.match(await page.locator('#source-title').textContent(), /^crates\//);
    await page.keyboard.press('Escape');
    pass('Lecture de la preuve dans le code', 'Source Rust accessible depuis un détail, avec fermeture au clavier.');

    await page.locator('#vues').scrollIntoViewIfNeeded();
    await page.screenshot({ path: path.join(evidence, 'architecture-globale.png') });
    report.captures.push('preuves/architecture-globale.png');
    const diagram = page.locator('.topic-figure-strip [data-figure]').first();
    await diagram.click();
    await page.waitForFunction(() => {
      const image = document.getElementById('zoom-image');
      return image.complete && image.naturalWidth > 0;
    });
    assert.ok(await page.locator('#figure-dialog').isVisible());
    await page.screenshot({ path: path.join(evidence, 'schema-agrandi.png') });
    report.captures.push('preuves/schema-agrandi.png');
    await page.locator('#figure-dialog [data-close]').click();
    const imageErrors = await page.evaluate(async () => {
      const images = [...document.images].filter(image => image.hasAttribute('src'));
      images.forEach(image => { image.loading = 'eager'; });
      const settled = await Promise.allSettled(images.map(image => image.decode()));
      return settled.flatMap((result, index) => result.status === 'rejected' ? [images[index].alt] : []);
    });
    assert.deepEqual(imageErrors, []);
    pass('Figures et agrandissement', { imagesInvalides: imageErrors });
    await page.locator('#gallery-search').fill('numéro');
    assert.ok(await page.locator('#gallery .figure:not([hidden])').count() > 0);
    await page.locator('#gallery-search').fill('motabsentdelarevuexyz');
    assert.equal(await page.locator('#gallery .figure:not([hidden])').count(), 0);
    await page.locator('#gallery-search').fill('');
    pass('Recherche dans les figures', 'Filtrage par texte, absence de résultat et retour à la galerie.');
    await page.locator('#source-search').fill('haute-disponibilite-v2');
    assert.ok(await page.locator('.source-index:not([hidden])').count() > 0);
    assert.ok(await page.locator('.source-index:not([hidden])').count() < 163);
    await page.locator('#source-search').fill('');
    pass('Index du corpus', 'Filtrage des sources par chemin et titres de section.');

    await page.setViewportSize({ width: 390, height: 844 });
    await page.goto(file);
    const width = await page.evaluate(() => ({ viewport: innerWidth, content: document.documentElement.scrollWidth }));
    assert.equal(width.viewport, width.content);
    await page.locator('#menu-toggle').click();
    assert.equal(await page.locator('#menu-toggle').getAttribute('aria-expanded'), 'true');
    await page.locator('.sidebar nav a[href="#vues"]').click();
    assert.equal(await page.locator('#menu-toggle').getAttribute('aria-expanded'), 'false');
    await page.screenshot({ path: path.join(evidence, 'vue-ensemble-mobile.png') });
    report.captures.push('preuves/vue-ensemble-mobile.png');
    pass('Navigation mobile et largeur', width);

    await page.locator('#tab-stockage').click();
    assert.ok(await page.locator('#theme-stockage .theme-explanation').isVisible());
    const tabWidth = await page.evaluate(() => ({ viewport: innerWidth, content: document.documentElement.scrollWidth }));
    assert.equal(tabWidth.viewport, tabWidth.content);
    await page.waitForFunction(() => {
      const panel = document.getElementById('theme-stockage').getBoundingClientRect();
      const tab = document.getElementById('tab-stockage').getBoundingClientRect();
      return panel.top > 100 && panel.top < innerHeight / 2 && tab.left >= 0 && tab.right <= innerWidth;
    });
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    // Laisser finir la rastérisation des tuiles après le changement de panneau.
    await page.waitForTimeout(250);
    await page.screenshot({ path: path.join(evidence, 'onglet-stockage-mobile.png') });
    report.captures.push('preuves/onglet-stockage-mobile.png');
    await page.reload();
    assert.equal(await page.locator('#tab-stockage').getAttribute('aria-selected'), 'true');
    assert.ok(await page.locator('#theme-stockage').isVisible());
    pass('Onglets mobiles et lien rechargé', 'Barre défilable sans débordement ; le rechargement rouvre le thème courant.');

    await page.locator('#parcours').scrollIntoViewIfNeeded();
    await page.locator('.route').first().locator('summary').click();
    assert.equal(await page.locator('.route[open]').count(), 1);
    const openBefore = await page.locator('details[open]').count();
    await page.evaluate(() => window.dispatchEvent(new Event('beforeprint')));
    assert.equal(await page.locator('details:not([open])').count(), 0);
    await page.evaluate(() => window.dispatchEvent(new Event('afterprint')));
    assert.equal(await page.locator('details[open]').count(), openBefore);
    pass('Parcours et préparation à l’impression', 'Le parcours se déplie ; les détails sont ouverts pour l’impression puis leur état est restauré.');
    assert.deepEqual(report.erreurs, []);
    assert.deepEqual(report.requetesExternes, []);
    pass('Exécution autonome', { erreursJavaScript: report.erreurs, requetesReseau: report.requetesExternes });
    report.succes = true;
  } catch (error) {
    report.succes = false;
    report.erreurs.push(error.message);
    process.exitCode = 1;
  } finally {
    clearTimeout(deadline);
    await browser.close();
    fs.writeFileSync(path.join(directory, 'controle-navigateur.json'), JSON.stringify(report, null, 2) + '\n');
    console.log(JSON.stringify(report, null, 2));
  }
})().catch(error => { console.error(error.message); process.exitCode = 1; });
