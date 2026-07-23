/* The library's search query lives outside the page because two surfaces
   drive it: the desktop's top search field and the mobile bottom search bar
   that the layout swaps in for the dock, iOS 26 style. */
class LibrarySearchState {
  query = $state("");
  open = $state(false);

  openSearch(): void {
    this.open = true;
  }

  close(): void {
    this.open = false;
    this.query = "";
  }
}

export const librarySearch = new LibrarySearchState();
