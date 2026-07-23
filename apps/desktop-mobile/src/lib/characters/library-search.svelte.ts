/* The library query and presentation state survive the header's compact /
   large-title transition, while the page owns the actual search surface. */
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
