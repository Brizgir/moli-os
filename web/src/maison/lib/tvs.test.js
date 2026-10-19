import { test } from 'node:test';
import assert from 'node:assert/strict';
import { roomTvs, hasRemote, chosen } from './tvs.js';

const remote = { key: 'key', access: { read: false, write: true } };
const tv = (id, room, name) => ({ id, native_name: name, native_room: room, points: [remote, { key: 'power', access: { read: true, write: true } }] });
const lamp = (id, room) => ({ id, native_room: room, points: [{ key: 'on', access: { read: true, write: true } }] });

// A living room with a TV and a decoder plugged into it, a bedroom TV.
const devices = [
  tv('hisense:salon', 'Salon', 'Télé du salon'),
  tv('orange:stb', 'Salon', 'Décodeur'),
  tv('androidtv:chambre', 'Chambre', 'Télé de la chambre'),
  lamp('hue:1', 'Salon'),
];
const opts = { roomOf: (d) => d.native_room, nameOf: (id) => devices.find((d) => d.id === id)?.native_name ?? id };

test('a device with a remote takes key presses', () => {
  assert.equal(hasRemote(devices[0]), true);
  assert.equal(hasRemote(devices[3]), false);
  assert.equal(hasRemote({ points: [{ key: 'key', access: { read: true, write: false } }] }), false);
});

test("the living room: the layout's TV first, then the room's other ones", () => {
  assert.deepEqual(roomTvs(devices, { ...opts, first: 'hisense:salon' }), ['hisense:salon', 'orange:stb']);
  // Chosen the other way round in the layout: its order wins.
  assert.deepEqual(roomTvs(devices, { ...opts, first: 'orange:stb' }), ['orange:stb', 'hisense:salon']);
  // `also`: one more, even from another room.
  assert.deepEqual(roomTvs(devices, { ...opts, first: 'hisense:salon', also: ['androidtv:chambre'] }), ['hisense:salon', 'androidtv:chambre', 'orange:stb']);
});

test('a room without a chosen TV: its TVs by name', () => {
  assert.deepEqual(roomTvs(devices, { ...opts, room: 'Salon' }), ['orange:stb', 'hisense:salon']);
  assert.deepEqual(roomTvs(devices, { ...opts, room: 'Chambre' }), ['androidtv:chambre']);
  assert.deepEqual(roomTvs(devices, { ...opts, room: 'Cuisine' }), []);
  // A layout TV that does not exist (yet): ignored.
  assert.deepEqual(roomTvs(devices, { ...opts, first: 'tele:gone', room: 'Chambre' }), ['androidtv:chambre']);
});

test('hidden TVs stay out', () => {
  assert.deepEqual(roomTvs(devices, { ...opts, room: 'Salon', hidden: (id) => id === 'orange:stb' }), ['hisense:salon']);
});

test('the picked TV stays while it is there', () => {
  assert.equal(chosen(['a', 'b'], 'b'), 'b');
  assert.equal(chosen(['a', 'b'], 'c'), 'a');
  assert.equal(chosen([], 'c'), null);
});
