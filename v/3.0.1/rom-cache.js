// The player's ROM, cached in IndexedDB by the randomizer page.
//
// One module so every page that draws from the ROM (the randomizer, the CHR
// picker and viewer, the maze tracker) opens the same database the same way.
// What is stored is the bytes the player supplied — the vanilla input, never a
// randomized ROM — so art the randomizer writes is not in it.

const DB_NAME = "smb3-rs";
const DB_STORE = "rom";
const ROM_KEY = "data";

export function openDb() {
	return new Promise((resolve, reject) => {
		const req = indexedDB.open(DB_NAME, 1);
		req.onupgradeneeded = () => req.result.createObjectStore(DB_STORE);
		req.onsuccess = () => resolve(req.result);
		req.onerror = () => reject(req.error);
	});
}

export async function saveRom(bytes) {
	const db = await openDb();
	const tx = db.transaction(DB_STORE, "readwrite");
	tx.objectStore(DB_STORE).put(bytes, ROM_KEY);
}

// Resolves to the cached bytes, or null when nothing has been cached.
export async function loadRom() {
	const db = await openDb();
	return new Promise((resolve) => {
		const tx = db.transaction(DB_STORE, "readonly");
		const req = tx.objectStore(DB_STORE).get(ROM_KEY);
		req.onsuccess = () => resolve(req.result || null);
		req.onerror = () => resolve(null);
	});
}

// Drop one key from the store. For cleaning up entries older versions left.
export async function deleteCached(key) {
	const db = await openDb();
	const tx = db.transaction(DB_STORE, "readwrite");
	tx.objectStore(DB_STORE).delete(key);
}
