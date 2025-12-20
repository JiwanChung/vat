import React from "react";

type Props = { title: string };

export function Banner({ title }: Props) {
  return <h1>{title}</h1>;
}

export const Card = ({ title }: Props) => {
  return <section>{title}</section>;
};
