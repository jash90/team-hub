/*
 * team-hub landing page. Everything here is an enhancement: with scripts off
 * a screenshot link opens the image and the commands are still selectable.
 *
 *  1. Lightbox: a screenshot link opens in a <dialog> instead of navigating.
 *     Escape closes (the dialog does that itself and gives focus back to the
 *     link), arrows move between screenshots, a click on the picture or the
 *     1:1 button switches between "fit" and actual size.
 *  2. Copy buttons on command blocks.
 */
(function () {
  'use strict';

  var polish = document.documentElement.lang === 'pl';
  var words = polish
    ? {
        close: 'Zamknij',
        previous: 'Poprzedni zrzut',
        next: 'Następny zrzut',
        actual: 'Rozmiar rzeczywisty',
        fit: 'Dopasuj do okna',
        of: ' z ',
        label: 'Zrzut ekranu w powiększeniu',
        copy: 'Kopiuj',
        copied: 'Skopiowano',
      }
    : {
        close: 'Close',
        previous: 'Previous screenshot',
        next: 'Next screenshot',
        actual: 'Actual size',
        fit: 'Fit to window',
        of: ' of ',
        label: 'Screenshot, enlarged',
        copy: 'Copy',
        copied: 'Copied',
      };

  function element(tag, className, text) {
    var node = document.createElement(tag);
    if (className) node.className = className;
    if (text) node.textContent = text;
    return node;
  }

  /* ---------- Lightbox ---------- */

  var links = Array.prototype.slice.call(document.querySelectorAll('.shot a.frame'));

  if (links.length && typeof HTMLDialogElement === 'function') {
    var dark = window.matchMedia('(prefers-color-scheme: dark)');
    var dialog = element('dialog', 'lightbox');
    var bar = element('div', 'bar');
    var count = element('span', 'count');
    var previous = element('button', '', '←');
    var next = element('button', '', '→');
    var size = element('button', '', '1:1');
    var close = element('button', '', '×');
    var stage = element('div', 'stage');
    var picture = element('img');
    var caption = element('p', 'caption');
    var current = 0;

    dialog.setAttribute('aria-label', words.label);
    previous.type = next.type = size.type = close.type = 'button';
    previous.setAttribute('aria-label', words.previous);
    next.setAttribute('aria-label', words.next);
    close.setAttribute('aria-label', words.close);
    count.setAttribute('aria-live', 'polite');
    stage.tabIndex = -1;

    bar.append(count, previous, next, size, close);
    stage.append(picture);
    dialog.append(bar, stage, caption);
    document.body.append(dialog);

    var setActual = function (on) {
      dialog.classList.toggle('actual', on);
      size.setAttribute('aria-label', on ? words.fit : words.actual);
      size.setAttribute('aria-pressed', on ? 'true' : 'false');
      if (on) stage.scrollTo(0, 0);
    };

    var show = function (index) {
      current = (index + links.length) % links.length;
      var link = links[current];
      var small = link.querySelector('img');
      var figure = link.closest('figure');
      var text = figure && figure.querySelector('figcaption');
      picture.alt = small ? small.alt : '';
      picture.src = (dark.matches && link.dataset.dark) || link.href;
      caption.textContent = text ? text.textContent.replace(/\s+/g, ' ').trim() : '';
      count.textContent = current + 1 + words.of + links.length;
      setActual(false);
    };

    links.forEach(function (link, index) {
      link.addEventListener('click', function (event) {
        // Leave "open in a new tab" and friends alone.
        if (event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) {
          return;
        }
        event.preventDefault();
        show(index);
        dialog.showModal();
        close.focus();
      });
    });

    previous.addEventListener('click', function () {
      show(current - 1);
    });
    next.addEventListener('click', function () {
      show(current + 1);
    });
    size.addEventListener('click', function () {
      setActual(!dialog.classList.contains('actual'));
    });
    picture.addEventListener('click', function () {
      setActual(!dialog.classList.contains('actual'));
    });
    close.addEventListener('click', function () {
      dialog.close();
    });
    // A click on the dark area around the picture closes, like a backdrop.
    stage.addEventListener('click', function (event) {
      if (event.target === stage) dialog.close();
    });
    dialog.addEventListener('keydown', function (event) {
      if (event.key === 'ArrowLeft') show(current - 1);
      else if (event.key === 'ArrowRight') show(current + 1);
    });
    dialog.addEventListener('close', function () {
      picture.removeAttribute('src');
      // The dialog returns focus by itself; this covers browsers that do not.
      var link = links[current];
      if (link && document.activeElement !== link) link.focus();
    });
  }

  /* ---------- Copy buttons ---------- */

  if (navigator.clipboard && navigator.clipboard.writeText) {
    Array.prototype.forEach.call(document.querySelectorAll('.code pre'), function (pre) {
      var commands = pre.querySelector('.p');
      if (!commands) return; // only blocks that hold commands, not examples
      var button = element('button', 'copy', words.copy);
      button.type = 'button';
      button.addEventListener('click', function () {
        var copy = pre.cloneNode(true);
        Array.prototype.forEach.call(copy.querySelectorAll('.p'), function (prompt) {
          prompt.remove();
        });
        // Join the continued lines, so that one paste is one command.
        var text = copy.textContent.replace(/\s*\\\n\s*/g, ' ').trim();
        navigator.clipboard.writeText(text).then(function () {
          button.textContent = words.copied;
          window.setTimeout(function () {
            button.textContent = words.copy;
          }, 1600);
        });
      });
      pre.parentNode.append(button);
    });
  }
})();
