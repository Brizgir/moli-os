import { test } from 'node:test';
import assert from 'node:assert/strict';
import { CARD_TYPES, SIZES, SPAN, screenFor, defaultLayout, defaultLayouts, resolve, names, slug, newId, move, shift, update, setHidden, add, remove, duplicate, withCards, addLayout, removeLayout, catalogue, keyOf, MAX_CARDS, MAX_LAYOUTS } from './layout.js';

test('the default is today\'s page: every card once, the main column two thirds wide', () => {
  const desktop = defaultLayout('desktop').cards;
  const phone = defaultLayout('phone').cards;
  assert.deepEqual(desktop.map((c) => c.type).sort(), phone.map((c) => c.type).sort());
  for (const c of desktop) assert.ok(CARD_TYPES.includes(c.type) && SIZES.includes(c.size) && /^[a-z0-9]{4,16}$/.test(c.id), JSON.stringify(c));
  assert.equal(desktop[0].type, 'sky');
  assert.equal(desktop.find((c) => c.type === 'heating').size, 'm');
  assert.equal(desktop.find((c) => c.type === 'doorbell').size, 's');
  assert.deepEqual(phone.slice(0, 4).map((c) => c.type), ['sky', 'modes', 'notices', 'heating']);
  assert.ok(phone.every((c) => c.size === 'l'));
  assert.deepEqual(Object.keys(defaultLayouts()), ['default']);
  assert.equal(SPAN.s + SPAN.m, SPAN.l, 'a third and two thirds make a row');
});

test('the phone below 768 px, the big screen from there', () => {
  assert.equal(screenFor(390), 'phone');
  assert.equal(screenFor(767), 'phone');
  assert.equal(screenFor(768), 'desktop');
});

test('mine before the house\'s before the default, and the active named layout', () => {
  const house = { default: { desktop: { cards: [{ id: 'aaaa', type: 'music', size: 'l' }] }, phone: { cards: [] } } };
  const mine = { default: { desktop: { cards: [{ id: 'bbbb', type: 'today', size: 's' }] }, phone: { cards: [] } }, soir: { name: 'Soir', desktop: { cards: [{ id: 'cccc', type: 'routines', size: 's' }] }, phone: { cards: [] } } };
  const none = resolve({}, 'desktop');
  assert.equal(none.stored, false);
  assert.equal(none.scope, null);
  assert.equal(none.cards.length, defaultLayout('desktop').cards.length);
  assert.equal(resolve({ house }, 'desktop').cards[0].id, 'aaaa');
  assert.equal(resolve({ house }, 'desktop').scope, 'house');
  assert.equal(resolve({ house, mine }, 'desktop').cards[0].id, 'bbbb');
  assert.equal(resolve({ house, mine }, 'desktop', 'soir').cards[0].id, 'cccc');
  assert.equal(resolve({ house, mine }, 'desktop', 'soir').active, 'soir');
  assert.equal(resolve({ house, mine }, 'desktop', 'matin').active, 'default', 'an unknown name: the default');
  assert.deepEqual(resolve({ house, mine }, 'phone').cards, []);
  assert.deepEqual(names(mine), [{ key: 'default', name: null }, { key: 'soir', name: 'Soir' }]);
});

test('a card moves, shifts among the shown ones, changes, hides and comes back', () => {
  const cards = [
    { id: 'aaaa', type: 'sky', size: 'l' },
    { id: 'bbbb', type: 'heating', size: 'm', hidden: true },
    { id: 'cccc', type: 'music', size: 's' },
    { id: 'dddd', type: 'today', size: 's' },
  ];
  assert.deepEqual(move(cards, 0, 2).map((c) => c.id), ['bbbb', 'cccc', 'aaaa', 'dddd']);
  assert.equal(move(cards, 1, 1), cards);
  assert.deepEqual(shift(cards, 'cccc', -1).map((c) => c.id), ['cccc', 'aaaa', 'bbbb', 'dddd'], 'past the hidden one');
  assert.deepEqual(shift(cards, 'aaaa', 1).map((c) => c.id), ['bbbb', 'cccc', 'aaaa', 'dddd']);
  assert.equal(shift(cards, 'aaaa', -1), cards, 'already first');
  assert.equal(shift(cards, 'dddd', 1), cards, 'already last');
  const changed = update(cards, 'cccc', { size: 'l', title: 'Son', options: { device: 'x:1' } });
  assert.deepEqual(changed[2], { id: 'cccc', type: 'music', size: 'l', title: 'Son', options: { device: 'x:1' } });
  assert.equal(update(changed, 'cccc', { options: { more: true } })[2].options.device, 'x:1', 'options merge');
  assert.equal(changed[0], cards[0], 'the others are the same objects');
  const hidden = setHidden(cards, 'aaaa', true);
  assert.deepEqual(hidden.map((c) => [c.id, !!c.hidden]), [['bbbb', true], ['cccc', false], ['dddd', false], ['aaaa', true]]);
  const back = setHidden(hidden, 'bbbb', false);
  assert.deepEqual(back.at(-1), { id: 'bbbb', type: 'heating', size: 'm', hidden: false }, 'shown again at the end');
});

test('a card is added, duplicated with its own id, removed; sixty at most', () => {
  const cards = [{ id: 'aaaa', type: 'sky', size: 'l' }];
  const added = add(cards, { type: 'room', options: { room: 'Salon' } });
  assert.equal(added.length, 2);
  assert.match(added[1].id, /^[a-z0-9]{8}$/);
  assert.equal(added[1].size, 'm');
  const twice = duplicate(added, added[1].id);
  assert.equal(twice.length, 3);
  assert.notEqual(twice[2].id, twice[1].id);
  assert.deepEqual(twice[2].options, { room: 'Salon' });
  assert.notEqual(twice[2].options, twice[1].options, 'a copy, not the same object');
  assert.equal(remove(twice, twice[2].id).length, 2);
  let many = [];
  for (let i = 0; i < MAX_CARDS; i++) many = add(many, { type: 'room' });
  assert.equal(add(many, { type: 'room' }).length, MAX_CARDS);
  const taken = ['abcdefgh'];
  assert.notEqual(newId(taken), 'abcdefgh');
});

test('a set of layouts: cards in place, named ones added and removed', () => {
  const set = withCards(null, 'default', 'phone', [{ id: 'aaaa', type: 'sky', size: 'l' }]);
  assert.equal(set.default.phone.cards[0].id, 'aaaa');
  assert.equal(set.default.desktop.cards.length, defaultLayout('desktop').cards.length, 'the other screen: the default');
  const { set: more, key } = addLayout(set, ' Le soir ');
  assert.equal(key, 'le-soir');
  assert.equal(more['le-soir'].name, 'Le soir');
  assert.equal(more['le-soir'].phone.cards[0].id, 'aaaa', 'copied from the default');
  assert.equal(addLayout(more, 'Le soir').key, null, 'taken');
  assert.equal(addLayout(more, '   ').key, null);
  assert.equal(slug('Défaut !'), 'defaut');
  assert.equal(slug('default'), 'default-2');
  const edited = withCards(more, 'le-soir', 'desktop', []);
  assert.deepEqual(edited['le-soir'].desktop.cards, []);
  assert.equal(edited['le-soir'].phone.cards[0].id, 'aaaa');
  assert.deepEqual(Object.keys(removeLayout(edited, 'le-soir')), ['default']);
  assert.equal(removeLayout(edited, 'default'), edited, 'never the default');
  let full = set;
  for (let i = 1; i < MAX_LAYOUTS; i++) full = addLayout(full, `L${i}`).set;
  assert.equal(addLayout(full, 'Encore').key, null, 'a dozen at most');
});

test('the catalogue says what is there, hidden, or to add', () => {
  const facts = {
    rooms: [{ name: 'Salon' }, { name: 'Bureau' }],
    devices: [
      { id: 'alexa:1', name: 'Echo', kind: 'speaker' },
      { id: 'tuya:d', name: 'Diffuseur', kind: 'diffuser' },
      { id: 'hue:1', name: 'Lampe', kind: 'light' },
    ],
  };
  const cards = [
    { id: 'aaaa', type: 'room', options: { room: 'Salon' } },
    { id: 'bbbb', type: 'speaker', options: { device: 'alexa:1' }, hidden: true },
    { id: 'cccc', type: 'music' },
  ];
  const c = catalogue(facts, cards);
  assert.deepEqual(c.rooms.map((r) => [r.label, r.present]), [['Salon', 'shown'], ['Bureau', null]]);
  assert.deepEqual(c.devices.map((d) => [d.type, d.label, d.present]), [['speaker', 'Echo', 'hidden'], ['diffuser', 'Diffuseur', null]]);
  assert.equal(c.house.find((h) => h.type === 'music').present, 'shown');
  assert.equal(c.house.find((h) => h.type === 'presence').present, null);
  assert.equal(keyOf({ type: 'room', options: { room: 'Salon' } }), 'room:Salon');
  assert.equal(keyOf({ type: 'sky' }), 'sky:');
});
