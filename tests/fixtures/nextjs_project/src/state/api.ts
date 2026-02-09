import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";

export const postsApi = createApi({
    reducerPath: "postsApi",
    baseQuery: fetchBaseQuery({ baseUrl: "/api" }),
    endpoints: (builder) => ({
        getPosts: builder.query({ query: () => "/posts" }),
        getUser: builder.query({ query: (id: string) => `/users/${id}` }),
    }),
});

export const { useGetPostsQuery, useGetUserQuery } = postsApi;
