use embedded_graphics::{
    prelude::{PixelColor, Point, Transform},
    primitives::Rectangle,
};
use embedded_layout::{
    view_group::{ViewGroup, ViewGroupHelper},
    View,
};

use crate::table::{DisplayTable, DisplayTableItem};

// TODO: Replace with Vec and ViewGroup macro
impl<T> View for DisplayTableItem<T> {
    fn translate_impl(&mut self, by: Point) {
        View::translate_mut(&mut self.bounds, by);
    }

    fn bounds(&self) -> Rectangle {
        self.bounds
    }
}

impl<T, C> ViewGroup for &mut DisplayTable<T, C>
where
    C: PixelColor,
{
    fn len(&self) -> usize {
        self.items.len()
    }

    fn at(&self, idx: usize) -> &dyn View {
        self.items.get(idx).expect("Index out of bounds")
    }

    fn at_mut(&mut self, idx: usize) -> &mut dyn View {
        self.items.get_mut(idx).expect("Index out of bounds")
    }
}

impl<T, C> ViewGroup for DisplayTable<T, C>
where
    C: PixelColor,
{
    fn len(&self) -> usize {
        self.items.len()
    }

    fn at(&self, idx: usize) -> &dyn View {
        self.items.get(idx).expect("Index out of bounds")
    }

    fn at_mut(&mut self, idx: usize) -> &mut dyn View {
        self.items.get_mut(idx).expect("Index out of bounds")
    }
}

impl<T, C> View for &mut DisplayTable<T, C>
where
    C: PixelColor,
{
    fn translate_impl(&mut self, by: Point) {
        ViewGroupHelper::translate(self, by);
    }

    fn bounds(&self) -> Rectangle {
        ViewGroupHelper::bounds(self)
    }
}

impl<T, C: PixelColor> embedded_graphics::geometry::OriginDimensions for DisplayTable<T, C> {
    fn size(&self) -> embedded_graphics::prelude::Size {
        ViewGroupHelper::bounds(self).size
    }
}

impl<T, C: PixelColor> Transform for DisplayTable<T, C> {
    fn translate(&self, _by: Point) -> Self {
        todo!()
    }

    fn translate_mut(&mut self, _by: Point) -> &mut Self {
        todo!()
    }
}
