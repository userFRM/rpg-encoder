export const selectUser = (state: any) => state.auth.user;

export const selectIsLoggedIn = (state: any) => state.auth.user !== null;

export const selectAuthLoading = (state: any) => state.auth.loading;
